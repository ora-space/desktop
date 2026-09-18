mod config;
mod inspection;
pub use config::{CloneConfig, CloneSsh};

use crate::{Clock, Error, ManagedGitRunner, Node, NodeState, WorktreeGit, WriteGuard};
use gitlancer::git::branch_clone::build_branch_clone_command;
use ora_node_db::{CloneExecution, ClonePhase, CloneProgress, CloneTarget};
use ora_node_protocol::*;
use ora_utils::path::DirectoryIdentity;
use serde::{Deserialize, Serialize};
use std::{fs, os::unix::fs::DirBuilderExt};

/// Both parent and leaf identities must survive restart; path text alone is never ownership proof.
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct DirectoryEvidence {
    root: DirectoryIdentity,
    target: DirectoryIdentity,
}

impl<W: WriteGuard, C: Clock> Node<gitlancer::Git<ManagedGitRunner<W>>, W, C> {
    /// Enables acquisition under a separately injected existing root without modifying user permissions.
    pub fn configure_clone(&mut self, mut config: CloneConfig) -> Result<(), Error> {
        let mut protected = vec![self.home_directory.clone()];
        for binding in &self.repositories {
            protected.push(binding.worktree_root.clone());
            protected.push(std::path::PathBuf::from(
                binding.main_workspace.path.as_str(),
            ));
        }
        config.validate(&protected, self.git.runner().process_config())?;
        self.repository_config = Some(config);
        Ok(())
    }

    /// Deduplicates before reading configuration; every new side effect follows durable acceptance.
    pub fn submit_clone(
        &mut self,
        command: CloneRepositoryMessage,
    ) -> Result<ExecutionStatus, Error> {
        command.validate()?;
        if command.payload.spec.node_id != self.identity.node_id {
            return Err(ora_node_db::Error::NodeMismatch.into());
        }
        if let Some(record) = self
            .database
            .find_clone(&command.operation_id, &command.execution_id)?
        {
            if record.command != command {
                return Err(ora_node_db::Error::IdentityConflict.into());
            }
            return Ok(ExecutionStatus {
                node: self.identity.clone(),
                state: record.progress.state(),
            });
        }
        if !self.git.accepting_work() {
            return Err(Error::Stopping);
        }
        let config = self
            .repository_config
            .as_ref()
            .ok_or_else(|| Error::Configuration("clone is not configured".into()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let target = CloneTarget {
            repository_id: RepositoryId::new(id.clone()),
            root: config.repository_root.clone(),
            path: config.repository_root.join(id),
        };
        let record = self.database.accept_clone(&command, &target)?;
        self.state = NodeState::RecoveryPending;
        self.drive_clone(record)?;
        self.refresh_clone_state()?;
        Ok(ExecutionStatus {
            node: self.identity.clone(),
            state: self
                .database
                .execution_state(&command.operation_id, &command.execution_id)?,
        })
    }

    /// Reconciles original clone attempts without blocking independently reserved destinations.
    pub fn recover_clones(&mut self) -> Result<NodeState, Error> {
        for record in self.database.recoverable_clones()? {
            if !self.git.accepting_work() {
                break;
            }
            self.drive_clone(record)?;
        }
        self.refresh_clone_state()?;
        Ok(self.state)
    }

    /// Updates admission visibility without confusing an unknown clone with an unknown Worktree.
    fn refresh_clone_state(&mut self) -> Result<(), Error> {
        self.state = if self.database.recoverable()?.is_empty()
            && self.database.recoverable_clones()?.is_empty()
        {
            NodeState::Ready
        } else {
            NodeState::RecoveryPending
        };
        Ok(())
    }

    /// Advances only proven phases; any ambiguity keeps the target and original execution reserved.
    fn drive_clone(&mut self, mut record: CloneExecution) -> Result<(), Error> {
        let phase = match &record.progress {
            CloneProgress::Pending(phase) | CloneProgress::Unknown(phase) => phase.clone(),
            CloneProgress::Completed(_) => return Ok(()),
        };
        // Process responsibility is reconciled even if configuration or filesystem ownership changed.
        let recovered = if matches!(phase, ClonePhase::Dispatched { .. }) {
            Some(self.git.runner().recover_clone(&record))
        } else {
            None
        };
        let Some(config) = self.repository_config.clone() else {
            return Ok(());
        };
        if record.target.root != config.repository_root {
            return self.unknown_clone(&record);
        }
        if matches!(
            record.progress,
            CloneProgress::Unknown(ClonePhase::Reserved)
        ) {
            return Ok(());
        }
        if phase == ClonePhase::Reserved {
            // Recheck topology at use time: configuration may have outlived a replaced root.
            // This is accidental-replacement protection, not a sandbox against trusted owners.
            if ora_utils::path::open_trusted_path(
                &record.target.root,
                self.git.runner().process_config().expected_uid,
                ora_utils::path::TrustedPathKind::Directory,
            )
            .is_err()
            {
                return self.unknown_clone(&record);
            }
            match fs::DirBuilder::new()
                .mode(/*mode*/ 0o700)
                .create(&record.target.path)
            {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return self.unknown_clone(&record);
                }
                Err(_) => {
                    return self.fail_clone(
                        &record,
                        CloneFailureCode::OperationFailed,
                        CloneResidual::NoDirectory {},
                    );
                }
            }
            let evidence = (|| -> Result<String, Box<dyn std::error::Error>> {
                fs::File::open(&record.target.path)?.sync_all()?;
                fs::File::open(&record.target.root)?.sync_all()?;
                Ok(serde_json::to_string(&DirectoryEvidence {
                    root: DirectoryIdentity::read(&record.target.root)?,
                    target: DirectoryIdentity::read(&record.target.path)?,
                })?)
            })();
            let Ok(identity) = evidence else {
                return self.unknown_clone(&record);
            };
            record = self.database.advance_clone(
                &record,
                CloneProgress::Pending(ClonePhase::DirectoryCreated { identity }),
            )?;
        }
        let identity = match &record.progress {
            CloneProgress::Pending(
                ClonePhase::DirectoryCreated { identity } | ClonePhase::Dispatched { identity },
            )
            | CloneProgress::Unknown(
                ClonePhase::DirectoryCreated { identity } | ClonePhase::Dispatched { identity },
            ) => identity.clone(),
            CloneProgress::Pending(ClonePhase::Reserved)
            | CloneProgress::Unknown(ClonePhase::Reserved)
            | CloneProgress::Completed(_) => return Ok(()),
        };
        if !matches_directory(&record.target, &identity) {
            return self.unknown_clone(&record);
        }
        let newly_dispatched = matches!(
            record.progress,
            CloneProgress::Pending(ClonePhase::DirectoryCreated { .. })
                | CloneProgress::Unknown(ClonePhase::DirectoryCreated { .. })
        );
        if newly_dispatched {
            // DirectoryCreated has no network effects; a journal write failure is safely retryable.
            self.git.runner().prepare_clone(&record)?;
            record = self.database.advance_clone(
                &record,
                CloneProgress::Pending(ClonePhase::Dispatched {
                    identity: identity.clone(),
                }),
            )?;
        }
        let mut command = build_branch_clone_command(
            record.command.payload.spec.repository.as_str(),
            record.command.payload.spec.branch.as_str(),
            &record.target.path,
            &record.target.root,
            config.environment()?,
        );
        config.constrain(&mut command);
        let outcome = match recovered {
            None | Some(Ok(crate::managed::CloneRecovery::Absent)) => {
                self.git.runner().execute_clone(&record, &command)
            }
            Some(Ok(crate::managed::CloneRecovery::Exited(code))) => Ok(Some(code)),
            Some(Ok(crate::managed::CloneRecovery::Unknown)) => Ok(None),
            Some(Err(error)) => Err(error),
        };
        let Ok(Some(code)) = outcome else {
            return self.unknown_clone(&record);
        };
        if !matches_directory(&record.target, &identity) {
            return self.unknown_clone(&record);
        }
        let residual = CloneResidual::Retained {
            repository_id: record.target.repository_id.clone(),
            path: NodePath::new(
                record
                    .target
                    .path
                    .to_str()
                    .ok_or_else(|| Error::Configuration("non-UTF-8 clone target".into()))?,
            ),
        };
        if code != 0 {
            return self.fail_clone(&record, CloneFailureCode::OperationFailed, residual);
        }
        let inspected = inspection::verify(self.git.runner(), &record, &config);
        if !matches_directory(&record.target, &identity) {
            return self.unknown_clone(&record);
        }
        match inspected {
            Ok(Some(commit)) => self
                .database
                .complete_clone(
                    &record,
                    CloneExecutionResult::CloneReady(CloneReady {
                        node: self.identity.clone(),
                        spec: record.command.payload.spec.clone(),
                        repository_id: record.target.repository_id.clone(),
                        path: NodePath::new(record.target.path.to_str().ok_or_else(|| {
                            Error::Configuration("non-UTF-8 clone target".into())
                        })?),
                        commit,
                    }),
                )
                .map_err(Error::from),
            Ok(None) => self.fail_clone(&record, CloneFailureCode::BranchNotFound, residual),
            Err(_) => self.unknown_clone(&record),
        }
    }

    /// Persists uncertainty without allowing another clone or deleting an ambiguous directory.
    fn unknown_clone(&mut self, record: &CloneExecution) -> Result<(), Error> {
        if let CloneProgress::Pending(phase) = &record.progress {
            self.database
                .advance_clone(record, CloneProgress::Unknown(phase.clone()))?;
        }
        Ok(())
    }

    /// Records a definitive failure with structural residue information rather than raw Git output.
    fn fail_clone(
        &mut self,
        record: &CloneExecution,
        failure: CloneFailureCode,
        residual: CloneResidual,
    ) -> Result<(), Error> {
        self.database.complete_clone(
            record,
            CloneExecutionResult::CloneFailed(CloneFailed {
                node: self.identity.clone(),
                spec: record.command.payload.spec.clone(),
                failure,
                residual,
            }),
        )?;
        Ok(())
    }
}

/// Rejects missing/replaced parent or target directories before either dispatch or fact inspection.
fn matches_directory(target: &CloneTarget, identity: &str) -> bool {
    let (Ok(saved), Ok(root), Ok(directory)) = (
        serde_json::from_str::<DirectoryEvidence>(identity),
        DirectoryIdentity::read(&target.root),
        DirectoryIdentity::read(&target.path),
    ) else {
        return false;
    };
    saved
        == DirectoryEvidence {
            root,
            target: directory,
        }
}

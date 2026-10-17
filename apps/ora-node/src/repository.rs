mod config;
mod drive;
mod inspection;
mod snapshot;
pub use config::CloneConfig;
pub(crate) use drive::{CloneStep, CloneStepResult};

use crate::{
    Clock, Error, ManagedGitRunner, Node, NodeState, WorktreeGit, WriteGuard,
    managed::AttemptSettlement,
};
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
        // Nothing runs yet, so every transport repository belongs to an earlier process: the
        // work that used it finished, or resumes from durable state (a clone's next fetch pass,
        // a delivery's frozen plan, a restore that ended with its session). Only clones whose
        // workload sequence still has to read the snapshot keep theirs.
        let keep: Vec<_> = self
            .database
            .recoverable_clones()?
            .iter()
            .filter(|record| snapshot::reads_transport(record))
            .map(|record| snapshot::transport_path(&config, record))
            .collect();
        crate::git_server::purge_transports(&config.repository_root, &keep).map_err(|error| {
            Error::Configuration(format!(
                "transport repositories could not be cleared: {error}"
            ))
        })?;
        self.repository_config = Some(config);
        Ok(())
    }

    /// The clone configuration, whose repository root also holds the transport repositories.
    pub(crate) fn clone_config(&self) -> Option<&CloneConfig> {
        self.repository_config.as_ref()
    }

    /// Lets clone, delivery, restore and `ora-git` reach the Git Server with the deployment's
    /// git-access identity. Without it the Node declares none of them.
    pub fn configure_git_access(&mut self, config: crate::GitAccessConfig) {
        self.git_access = Some(config);
    }

    /// Replaces the pauses between Git Server attempts. Deployments keep the ADR's 5 and 30
    /// seconds; tests that exhaust the attempts shorten them.
    pub fn set_git_server_retry_delays(&mut self, delays: [std::time::Duration; 2]) {
        self.git_server_retry = crate::git_server::RetryPolicy { delays };
    }

    /// Node-identity Git Server access, available once both clone and git-access are configured:
    /// the clone search path is the `PATH` its Git runs with.
    pub(crate) fn git_server(&self) -> Option<crate::git_server::GitServerAccess> {
        let (Some(config), Some(material)) = (&self.repository_config, &self.git_access) else {
            return None;
        };
        let search_path = std::env::join_paths(&config.search_path)
            .ok()?
            .into_string()
            .ok()?;
        Some(crate::git_server::GitServerAccess::new(
            material.clone(),
            search_path,
            self.git_server_retry,
        ))
    }

    /// Revision delivery runs Git in the checkouts clone created, so it inherits clone's hardened
    /// environment and per-command settings; without clone configuration there is no checkout.
    pub(crate) fn delivery_git_policy(&self) -> Result<Option<crate::revision::GitPolicy>, Error> {
        let Some(config) = &self.repository_config else {
            return Ok(None);
        };
        let mut probe = gitlancer::GitCommand::new(
            std::path::PathBuf::new(),
            Vec::new(),
            gitlancer::GitEnv::default(),
            gitlancer::GitIntent::Mutating,
        );
        config.constrain(&mut probe);
        Ok(Some(crate::revision::GitPolicy {
            env: config.environment()?,
            config_args: probe.args,
        }))
    }

    /// Deduplicates before reading configuration; every new side effect follows durable acceptance.
    pub fn submit_clone(
        &mut self,
        command: CloneRepositoryMessage,
    ) -> Result<ExecutionStatus, Error> {
        let (status, fresh) = self.reserve_clone(&command)?;
        if let Some(record) = fresh {
            self.drive_clone(record)?;
            self.refresh_clone_state()?;
            return Ok(ExecutionStatus {
                node: self.identity.clone(),
                state: self
                    .database
                    .execution_state(&command.operation_id, &command.execution_id)?,
            });
        }
        Ok(status)
    }

    /// Separates durable admission from blocking Git so revoking a session never waits for a clone.
    pub(crate) fn reserve_clone(
        &mut self,
        command: &CloneRepositoryMessage,
    ) -> Result<(ExecutionStatus, Option<CloneExecution>), Error> {
        self.reserve_clone_with_control(command, None)
    }

    pub(crate) fn reserve_controlled_clone(
        &mut self,
        command: &CloneRepositoryMessage,
        permit: &RuntimeBinding,
    ) -> Result<(ExecutionStatus, Option<CloneExecution>), Error> {
        self.reserve_clone_with_control(command, Some(permit))
    }

    fn reserve_clone_with_control(
        &mut self,
        command: &CloneRepositoryMessage,
        permit: Option<&RuntimeBinding>,
    ) -> Result<(ExecutionStatus, Option<CloneExecution>), Error> {
        command.validate()?;
        if command.payload.spec.node_id != self.identity.node_id {
            return Err(ora_node_db::Error::NodeMismatch.into());
        }
        if let Some(record) = self
            .database
            .find_clone(&command.operation_id, &command.execution_id)?
        {
            if record.command != *command {
                return Err(ora_node_db::Error::IdentityConflict.into());
            }
            return Ok((
                ExecutionStatus {
                    node: self.identity.clone(),
                    state: record.progress.state(),
                },
                None,
            ));
        }
        if !self.git.accepting_work() {
            return Err(Error::Stopping);
        }
        let config = self
            .repository_config
            .as_ref()
            .filter(|_| self.git_access.is_some())
            .ok_or_else(|| Error::Configuration("clone is not configured".into()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let target = CloneTarget {
            repository_id: RepositoryId::new(id.clone()),
            root: config.repository_root.clone(),
            path: config.repository_root.join(id),
        };
        let record = if let Some(permit) = permit {
            self.database
                .accept_controlled_clone(command, &target, permit)?
        } else {
            self.database.accept_clone(command, &target)?
        };
        self.state = NodeState::RecoveryPending;
        Ok((
            ExecutionStatus {
                node: self.identity.clone(),
                state: record.progress.state(),
            },
            Some(record),
        ))
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
    pub(crate) fn refresh_clone_state(&mut self) -> Result<(), Error> {
        self.state = if self.database.recoverable()?.is_empty()
            && self.database.recoverable_clones()?.is_empty()
        {
            NodeState::Ready
        } else {
            NodeState::RecoveryPending
        };
        Ok(())
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
        // The transport repository is only needed until the clone ends, whatever the outcome.
        if let Some(config) = &self.repository_config {
            crate::git_server::remove_transport(&snapshot::transport_path(config, record));
        }
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

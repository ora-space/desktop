//! The clone state machine split at every host interaction.
//!
//! The thread that owns the Node database advances an execution through its durable phases until
//! the next step needs the host, then hands that step out. Whoever performs it only talks to the
//! host and returns what it observed; the owner persists the observation and decides what follows.
//! Every step's input is durable before it leaves, so an executor that disappears mid-step leaves
//! nothing a restart cannot rebuild from the database.
use super::*;
use crate::managed::CloneHost;
use ora_node_db::ProcessAttempt;
use std::io;

/// One host interaction a clone needs next; everything it depends on is already durable.
pub(crate) struct CloneStep {
    record: CloneExecution,
    effect: CloneEffect,
}

enum CloneEffect {
    /// Start the recorded attempt and settle it.
    Dispatch(ProcessAttempt),
    /// Settle an attempt an earlier pass or process recorded, without starting anything.
    Settle(ProcessAttempt),
    /// Read the checkout facts after Git exited successfully.
    Inspect(CloneConfig),
}

/// A performed step, carrying the host's answer back to the database owner.
pub(crate) struct CloneStepResult {
    record: CloneExecution,
    outcome: StepOutcome,
}

enum StepOutcome {
    Dispatched(ProcessAttempt, io::Result<AttemptSettlement>),
    Settled(ProcessAttempt, io::Result<AttemptSettlement>),
    Inspected(io::Result<Option<CommitId>>),
}

impl CloneStep {
    /// The execution this step belongs to; at most one step per execution is ever outstanding.
    pub(crate) fn execution(&self) -> &ExecutionId {
        &self.record.command.execution_id
    }

    /// Performs the host interaction without touching the Node database or the Node home.
    pub(crate) fn perform(self, host: &CloneHost) -> CloneStepResult {
        let outcome = match self.effect {
            CloneEffect::Dispatch(attempt) => {
                let settlement = host.dispatch(&attempt);
                StepOutcome::Dispatched(attempt, settlement)
            }
            CloneEffect::Settle(attempt) => {
                let settlement = host.settle(&attempt);
                StepOutcome::Settled(attempt, settlement)
            }
            CloneEffect::Inspect(config) => {
                StepOutcome::Inspected(inspection::verify(host, &self.record, &config))
            }
        };
        CloneStepResult {
            record: self.record,
            outcome,
        }
    }
}

impl CloneStepResult {
    /// The execution whose step produced this result.
    pub(crate) fn execution(&self) -> &ExecutionId {
        &self.record.command.execution_id
    }
}

impl<W: WriteGuard, C: Clock> Node<gitlancer::Git<ManagedGitRunner<W>>, W, C> {
    /// Drives one clone to its next resting phase on the calling thread.
    pub(super) fn drive_clone(&mut self, record: CloneExecution) -> Result<(), Error> {
        let mut step = self.begin_clone(record)?;
        while let Some(next) = step {
            let result = next.perform(self.git.runner().clone_host());
            step = self.resume_clone(result)?;
        }
        Ok(())
    }

    /// Advances an execution's durable phases until it needs the host, or until it rests.
    ///
    /// `None` means the execution completed, became Unknown, or waits for configuration.
    pub(crate) fn begin_clone(
        &mut self,
        record: CloneExecution,
    ) -> Result<Option<CloneStep>, Error> {
        let dispatched = match &record.progress {
            CloneProgress::Pending(phase) | CloneProgress::Unknown(phase) => {
                matches!(phase, ClonePhase::Dispatched { .. })
            }
            CloneProgress::Completed(_) => return Ok(None),
        };
        if !dispatched {
            return self.continue_clone(record, None);
        }
        // Process responsibility is reconciled even if configuration or filesystem ownership
        // changed, so an existing Run is settled before anything else is checked. Absence of any
        // recorded Run proves no dispatch; an existing uncertain Run is never replaced.
        match self.git.runner().clone_attempts(&record) {
            Ok(attempts) => match attempts.as_slice() {
                [] => self.continue_clone(record, None),
                [attempt] => Ok(Some(CloneStep {
                    effect: CloneEffect::Settle(attempt.clone()),
                    record,
                })),
                // Each execution dispatches at most one Run; more cannot be attributed safely.
                [_, _, ..] => self.continue_clone(record, Some(Ok(AttemptSettlement::Unverified))),
            },
            Err(error) => self.continue_clone(record, Some(Err(error))),
        }
    }

    /// Persists what a performed step observed and returns the step that follows, if any.
    pub(crate) fn resume_clone(
        &mut self,
        result: CloneStepResult,
    ) -> Result<Option<CloneStep>, Error> {
        let CloneStepResult { record, outcome } = result;
        match outcome {
            StepOutcome::Dispatched(attempt, settlement) => {
                let settlement = self.persist_settlement(&attempt, settlement);
                self.settle_clone(record, settlement)
            }
            StepOutcome::Settled(attempt, settlement) => {
                let settlement = self.persist_settlement(&attempt, settlement);
                self.continue_clone(record, Some(settlement))
            }
            StepOutcome::Inspected(inspected) => {
                self.finish_clone(&record, inspected)?;
                Ok(None)
            }
        }
    }

    /// A settlement that cannot be journaled is as unproven as one the host could not give.
    fn persist_settlement(
        &self,
        attempt: &ProcessAttempt,
        settlement: io::Result<AttemptSettlement>,
    ) -> io::Result<AttemptSettlement> {
        let settlement = settlement?;
        self.git.runner().persist_settlement(attempt, settlement)?;
        Ok(settlement)
    }

    /// Advances only proven phases; any ambiguity keeps the target and original execution reserved.
    ///
    /// `recovered` is the settlement of the Run a previous dispatch left behind, if there was one;
    /// without it the execution may still need its first dispatch.
    fn continue_clone(
        &mut self,
        mut record: CloneExecution,
        recovered: Option<io::Result<AttemptSettlement>>,
    ) -> Result<Option<CloneStep>, Error> {
        let Some(config) = self.repository_config.clone() else {
            return Ok(None);
        };
        if record.target.root != config.repository_root {
            return self.unknown_clone(&record).map(|()| None);
        }
        let phase = match &record.progress {
            CloneProgress::Pending(phase) | CloneProgress::Unknown(phase) => phase.clone(),
            CloneProgress::Completed(_) => return Ok(None),
        };
        if matches!(
            record.progress,
            CloneProgress::Unknown(ClonePhase::Reserved)
        ) {
            return Ok(None);
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
                return self.unknown_clone(&record).map(|()| None);
            }
            match fs::DirBuilder::new()
                .mode(/*mode*/ 0o700)
                .create(&record.target.path)
            {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return self.unknown_clone(&record).map(|()| None);
                }
                Err(_) => {
                    return self
                        .fail_clone(
                            &record,
                            CloneFailureCode::OperationFailed,
                            CloneResidual::NoDirectory {},
                        )
                        .map(|()| None);
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
                return self.unknown_clone(&record).map(|()| None);
            };
            record = self.database.advance_clone(
                &record,
                CloneProgress::Pending(ClonePhase::DirectoryCreated { identity }),
            )?;
        }
        let Some(identity) = directory_identity(&record) else {
            return Ok(None);
        };
        if !matches_directory(&record.target, &identity) {
            return self.unknown_clone(&record).map(|()| None);
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
                CloneProgress::Pending(ClonePhase::Dispatched { identity }),
            )?;
        }
        if let Some(settlement) = recovered {
            return self.settle_clone(record, settlement);
        }
        let mut command = build_branch_clone_command(
            record.command.payload.spec.repository.as_str(),
            record.command.payload.spec.branch.as_str(),
            &record.target.path,
            &record.target.root,
            config.environment()?,
        );
        config.constrain(&mut command);
        // The attempt is durable before it leaves this thread, so a restart settles this Run
        // instead of dispatching another.
        match self.git.runner().record_clone_attempt(&record, &command) {
            Ok(attempt) => Ok(Some(CloneStep {
                record,
                effect: CloneEffect::Dispatch(attempt),
            })),
            Err(_) => self.unknown_clone(&record).map(|()| None),
        }
    }

    /// Turns a settled attempt into a business result, or into the inspection that success needs.
    fn settle_clone(
        &mut self,
        record: CloneExecution,
        settlement: io::Result<AttemptSettlement>,
    ) -> Result<Option<CloneStep>, Error> {
        let settlement = match settlement {
            Ok(AttemptSettlement::Unverified) | Err(_) => {
                return self.unknown_clone(&record).map(|()| None);
            }
            Ok(settlement) => settlement,
        };
        let (Some(identity), Some(config)) =
            (directory_identity(&record), self.repository_config.clone())
        else {
            return self.unknown_clone(&record).map(|()| None);
        };
        if !matches_directory(&record.target, &identity) {
            return self.unknown_clone(&record).map(|()| None);
        }
        // A terminated attempt never reached Git's own verdict: it cannot have succeeded, and the
        // caller may retry with a new execution instead of investigating a Git failure.
        let code = match settlement {
            AttemptSettlement::Exited(code) => code,
            AttemptSettlement::Terminated(_) => {
                let residual = retained(&record)?;
                return self
                    .fail_clone(&record, CloneFailureCode::Interrupted, residual)
                    .map(|()| None);
            }
            AttemptSettlement::Unverified => return self.unknown_clone(&record).map(|()| None),
        };
        if code != 0 {
            let residual = retained(&record)?;
            return self
                .fail_clone(&record, CloneFailureCode::OperationFailed, residual)
                .map(|()| None);
        }
        Ok(Some(CloneStep {
            record,
            effect: CloneEffect::Inspect(config),
        }))
    }

    /// Completes a successful exit from inspected facts, rechecking the directory they came from.
    fn finish_clone(
        &mut self,
        record: &CloneExecution,
        inspected: io::Result<Option<CommitId>>,
    ) -> Result<(), Error> {
        let Some(identity) = directory_identity(record) else {
            return self.unknown_clone(record);
        };
        if !matches_directory(&record.target, &identity) {
            return self.unknown_clone(record);
        }
        match inspected {
            Ok(Some(commit)) => self
                .database
                .complete_clone(
                    record,
                    CloneExecutionResult::CloneReady(CloneReady {
                        node: self.identity.clone(),
                        spec: record.command.payload.spec.clone(),
                        repository_id: record.target.repository_id.clone(),
                        path: target_path(record)?,
                        commit,
                    }),
                )
                .map_err(Error::from),
            Ok(None) => {
                let residual = retained(record)?;
                self.fail_clone(record, CloneFailureCode::BranchNotFound, residual)
            }
            Err(_) => self.unknown_clone(record),
        }
    }
}

/// The created directory's recorded identity, once the execution got that far.
fn directory_identity(record: &CloneExecution) -> Option<String> {
    match &record.progress {
        CloneProgress::Pending(
            ClonePhase::DirectoryCreated { identity } | ClonePhase::Dispatched { identity },
        )
        | CloneProgress::Unknown(
            ClonePhase::DirectoryCreated { identity } | ClonePhase::Dispatched { identity },
        ) => Some(identity.clone()),
        CloneProgress::Pending(ClonePhase::Reserved)
        | CloneProgress::Unknown(ClonePhase::Reserved)
        | CloneProgress::Completed(_) => None,
    }
}

/// Failures after dispatch keep the directory; its path is reported, never removed.
fn retained(record: &CloneExecution) -> Result<CloneResidual, Error> {
    Ok(CloneResidual::Retained {
        repository_id: record.target.repository_id.clone(),
        path: target_path(record)?,
    })
}

/// Protocol paths are UTF-8; a target that is not cannot be reported and is a configuration fault.
fn target_path(record: &CloneExecution) -> Result<NodePath, Error> {
    Ok(NodePath::new(record.target.path.to_str().ok_or_else(
        || Error::Configuration("non-UTF-8 clone target".into()),
    )?))
}

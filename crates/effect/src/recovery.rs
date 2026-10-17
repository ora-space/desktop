use crate::{
    EffectOperation, EffectRepository, EffectTargetId, LocalTimestamp, OperationArtifact,
    RecoveryAttemptClosure, RecoveryAttemptJournal, RecoveryOperationClosure, RepositoryError,
    ResourceAdapter, ResourceAdapterError, TargetRecoveryFinalization,
};
use ora_utils::clock::TimestampSource;
use thiserror::Error;

/// Closes one Target's quarantined journals using only exact adapter evidence.
///
/// Recovery never replays a mutation and never guesses: each quarantined operation must prove
/// either its exact planned state (the effect landed, so the journal finalizes and keeps its
/// ledger row) or its exact expected baseline (the effect never became durable, so the journal
/// closes without effect and grants nothing). An operation that proves neither keeps the
/// Target quarantined, because no evidence distinguishes drift from half-applied work.
pub struct EffectRecoverer<'a, Repository, Resource, Clock> {
    repository: &'a Repository,
    resource_adapter: &'a Resource,
    clock: &'a Clock,
}

impl<'a, Repository, Resource, Clock> EffectRecoverer<'a, Repository, Resource, Clock>
where
    Repository: EffectRepository,
    Resource: ResourceAdapter,
    Clock: TimestampSource,
{
    pub fn new(
        repository: &'a Repository,
        resource_adapter: &'a Resource,
        clock: &'a Clock,
    ) -> Self {
        Self {
            repository,
            resource_adapter,
            clock,
        }
    }

    /// Recovers one Target by proving every quarantined journal against external state.
    pub fn recover(&self, target: &EffectTargetId) -> Result<RecoveryOutcome, RecoveryError> {
        let now = LocalTimestamp::from_millis(self.clock.current_timestamp_millis());
        // The same scan every worker pass runs first: a leftover prepared/applied journal
        // without a live claim becomes explicit recovery state before any decision is made,
        // while work still covered by a valid lease is never quarantined.
        self.repository.quarantine_unfinished_operations(now)?;
        let Some(journal) = self.repository.load_target_recovery(target)? else {
            return Err(RecoveryError::TargetMissing(target.clone()));
        };
        if journal.attempts.is_empty()
            && !matches!(
                journal.target_status.phase(),
                crate::TargetPhase::RecoveryRequired(_)
            )
        {
            return Ok(RecoveryOutcome::AlreadyHealthy {
                target: target.clone(),
            });
        }

        let finalized_at = LocalTimestamp::from_millis(self.clock.current_timestamp_millis());
        let mut finalized = 0_usize;
        let mut closed_without_effect = 0_usize;
        let mut attempts = Vec::new();
        let mut artifacts = Vec::new();
        for RecoveryAttemptJournal {
            mut attempt,
            operations,
            artifacts: attempt_artifacts,
        } in journal.attempts
        {
            let mut closures = Vec::new();
            for operation in operations {
                match self.prove_closure(operation)? {
                    RecoveryOperationClosure::Finalized(operation) => {
                        finalized += 1;
                        closures.push(RecoveryOperationClosure::Finalized(operation));
                    }
                    RecoveryOperationClosure::ClosedWithoutEffect(operation) => {
                        closed_without_effect += 1;
                        closures.push(RecoveryOperationClosure::ClosedWithoutEffect(operation));
                    }
                }
            }
            attempt.finalize_recovered()?;
            attempts.push(RecoveryAttemptClosure {
                attempt,
                operations: closures,
            });
            artifacts.extend(attempt_artifacts);
        }
        self.repository
            .finalize_target_recovery(TargetRecoveryFinalization {
                target: target.clone(),
                attempts,
                finalized_at,
            })?;
        self.cleanup_artifacts(&artifacts)?;
        if finalized + closed_without_effect == 0 {
            return Ok(RecoveryOutcome::AlreadyHealthy {
                target: target.clone(),
            });
        }
        Ok(RecoveryOutcome::Recovered {
            target: target.clone(),
            finalized,
            closed_without_effect,
        })
    }

    /// Classifies one quarantined operation by proving its exact external state.
    fn prove_closure(
        &self,
        mut operation: EffectOperation,
    ) -> Result<RecoveryOperationClosure, RecoveryError> {
        let finalized_at = LocalTimestamp::from_millis(self.clock.current_timestamp_millis());
        if self.resource_adapter.verify(&operation).is_ok() {
            operation.finalize_recovered(finalized_at)?;
            return Ok(RecoveryOperationClosure::Finalized(operation));
        }
        if self.resource_adapter.verify_expected(&operation).is_ok() {
            operation.close_without_effect(finalized_at)?;
            return Ok(RecoveryOperationClosure::ClosedWithoutEffect(operation));
        }
        Err(RecoveryError::NotRecoverable {
            resource: operation.resource().clone(),
            operation: operation.identity().clone(),
        })
    }

    /// Deletes staging and backup artifacts after the durable recovery closure committed.
    ///
    /// Cleanup failures never reopen the closed journal: the artifact stays recorded as
    /// `CleanupFailed` exactly like an ordinary reconcile, and later passes retry it.
    fn cleanup_artifacts(&self, artifacts: &[OperationArtifact]) -> Result<(), RecoveryError> {
        for artifact in artifacts {
            match self.resource_adapter.cleanup(artifact) {
                Ok(receipt) => self
                    .repository
                    .complete_artifact_cleanup(&artifact.identity, receipt)?,
                Err(_) => {
                    self.repository
                        .mark_artifact_cleanup_failed(artifact.clone())?;
                }
            }
        }
        Ok(())
    }
}

/// Observable result of one Target recovery decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryOutcome {
    /// Quarantined journals were closed and the Target returned to ordinary scheduling.
    Recovered {
        target: EffectTargetId,
        finalized: usize,
        closed_without_effect: usize,
    },
    /// No quarantined journal remained, so recovery changed no durable state.
    AlreadyHealthy { target: EffectTargetId },
}

/// Reports why one Target's recovery could not close its quarantined journals.
#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Resource(#[from] ResourceAdapterError),
    #[error(transparent)]
    Status(#[from] crate::StatusTransitionError),
    #[error(transparent)]
    Operation(#[from] crate::OperationTransitionError),
    #[error("Effect Target {0} does not exist")]
    TargetMissing(EffectTargetId),
    #[error(
        "operation {operation} on Resource {resource} proves neither its exact planned nor expected state"
    )]
    NotRecoverable {
        resource: crate::EffectResourceId,
        operation: crate::EffectOperationId,
    },
}

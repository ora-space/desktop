use crate::BackendError;
use crate::effect_worker::EffectWorkerHandle;
use ora_application::EffectService;
use ora_contracts::{
    EmptyErrorParams, GetEffectTargetStatusRequest, GetEffectTargetStatusResponse, PublicError,
    RecoverEffectTargetRequest, RecoverEffectTargetResponse,
};
use ora_db::{RepositoryPool, SqliteEffectRepository};
use ora_effect::{EffectRecoverer, EffectTargetId, RecoveryError, RecoveryOutcome};
use ora_effect_skill::SkillDirectoryResourceAdapter;

#[cfg(test)]
mod tests;

/// Exposes persisted Effect status and the controlled recovery exit for quarantined Targets.
///
/// The worker, generation, and ordinary reconciliation control stay behind the worker
/// thread: this surface only reads status and closes journals by exact adapter evidence.
#[derive(Clone)]
pub struct Effects {
    pool: RepositoryPool,
    wake: EffectWorkerHandle,
}

impl Effects {
    /// Shares the worker's pool and wake handle; status reads spawn no reconciliation loop.
    pub(crate) fn new(pool: RepositoryPool, wake: EffectWorkerHandle) -> Self {
        Self { pool, wake }
    }

    /// Loads one Effect Target selected by opaque id or Workspace plus Agent identity.
    pub fn target_status(
        &self,
        request: GetEffectTargetStatusRequest,
    ) -> Result<GetEffectTargetStatusResponse, BackendError> {
        EffectService::new(SqliteEffectRepository::new(self.pool.clone()))
            .get_target_status(request)
            .map_err(|error| BackendError::internal("failed to load Effect Target status", error))
    }

    /// Closes one quarantined Target's journals by proving each against external state.
    ///
    /// A journal that proves neither its exact planned state nor its exact expected
    /// baseline keeps the Target quarantined instead of guessing; the caller sees a
    /// typed refusal and the send gate stays closed.
    pub fn recover_target(
        &self,
        request: RecoverEffectTargetRequest,
    ) -> Result<RecoverEffectTargetResponse, BackendError> {
        let repository = SqliteEffectRepository::new(self.pool.clone());
        let outcome = EffectRecoverer::new(
            &repository,
            &SkillDirectoryResourceAdapter,
            &crate::clock::SystemClock,
        )
        .recover(&EffectTargetId::new(request.target_id))
        .map_err(|error| match error {
            RecoveryError::TargetMissing(_) => BackendError::new(
                crate::error::ErrorClassification::NotFound,
                PublicError::EffectTargetNotFound(EmptyErrorParams {}),
                "Effect Target does not exist",
            ),
            RecoveryError::NotRecoverable {
                resource,
                operation,
            } => BackendError::new(
                crate::error::ErrorClassification::Unprocessable,
                PublicError::EffectTargetNotRecoverable(EmptyErrorParams {}),
                format!("operation {operation} on Resource {resource} proves neither exact state"),
            ),
            error @ (RecoveryError::Repository(_)
            | RecoveryError::Resource(_)
            | RecoveryError::Status(_)
            | RecoveryError::Operation(_)) => {
                BackendError::internal("failed to recover Effect Target", error)
            }
        })?;
        // The durable request is pending again; wake the worker so convergence starts
        // immediately instead of waiting for the periodic scan.
        self.wake.notify();
        let (recovered, finalized, closed_without_effect) = match outcome {
            RecoveryOutcome::Recovered {
                finalized,
                closed_without_effect,
                ..
            } => (true, finalized, closed_without_effect),
            RecoveryOutcome::AlreadyHealthy { .. } => (false, 0, 0),
        };
        Ok(RecoverEffectTargetResponse {
            recovered,
            finalized_operations: finalized,
            closed_without_effect_operations: closed_without_effect,
        })
    }
}

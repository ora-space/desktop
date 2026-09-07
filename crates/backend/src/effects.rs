use crate::BackendError;
use ora_application::EffectService;
use ora_contracts::{GetEffectTargetStatusRequest, GetEffectTargetStatusResponse};
use ora_db::{RepositoryPool, SqliteEffectRepository};

#[cfg(test)]
mod tests;

/// Exposes persisted Effect status without granting worker, generation, or reconciliation control.
#[derive(Clone)]
pub struct Effects {
    pool: RepositoryPool,
}

impl Effects {
    /// Uses the same pool as the worker; status queries do not spawn another reconciliation loop.
    pub(crate) fn new(pool: RepositoryPool) -> Self {
        Self { pool }
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
}

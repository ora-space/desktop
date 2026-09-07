use crate::TimestampSource;
use ora_effect::{EffectScopeId, LocalTimestamp};
use rusqlite::{Transaction, params};

/// Samples audit time only after the caller has acquired its write transaction.
///
/// The sample belongs to this transaction, not to a source revision or a worker pass. Updates
/// still preserve each row's own lower bound because the wall clock may move backwards.
pub(crate) struct EffectWriteContext {
    timestamp: LocalTimestamp,
}

impl EffectWriteContext {
    /// Requires an acquired transaction so lock wait cannot make the sample stale before entry.
    pub(crate) fn new(_transaction: &Transaction<'_>, clock: &impl TimestampSource) -> Self {
        Self {
            timestamp: LocalTimestamp::from_millis(clock.current_timestamp_millis()),
        }
    }

    /// Returns the shared audit sample for writes within this transaction.
    pub(crate) fn timestamp(&self) -> LocalTimestamp {
        self.timestamp
    }

    /// Creates a Scope in its Workspace transaction without inheriting historical catalog time.
    pub(crate) fn create_scope(
        &self,
        transaction: &Transaction<'_>,
        scope: &EffectScopeId,
    ) -> Result<(), crate::DatabaseError> {
        let EffectScopeId::Workspace(workspace) = scope;
        transaction.execute(
            "INSERT INTO effect_scopes (
                 id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at
             ) VALUES (?1, 'workspace', ?2, 'active', 0, ?3, ?3)",
            params![
                scope.storage_key(),
                workspace.as_ref(),
                self.timestamp.millis()
            ],
        )?;
        Ok(())
    }
}

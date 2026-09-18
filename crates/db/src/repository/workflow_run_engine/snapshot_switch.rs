use super::engine_repository_error_from_database;
use crate::repository::RepositoryPool;
use ora_application::RepositoryError;
use ora_domain::{WorkflowRunId, WorkflowRunStatus, WorkflowSnapshotId};
use rusqlite::{Transaction, TransactionBehavior, params};

/// Points a Failed/Cancelled run at another snapshot and stores the migrated payload.
///
/// Returns `false` when the run is missing or not in a resumable status.
pub(super) fn switch_run_snapshot(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    snapshot_id: &WorkflowSnapshotId,
    payload_json: &str,
    now: i64,
) -> Result<bool, RepositoryError> {
    pool.with_connection_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "UPDATE workflow_runs SET snapshot_id = ?2, payload = ?3, updated_at = ?4
             WHERE id = ?1 AND is_deleted = 0 AND run_status IN (?5, ?6)",
            params![
                run_id.as_ref(),
                snapshot_id.as_ref(),
                payload_json,
                now,
                WorkflowRunStatus::Failed.database_value(),
                WorkflowRunStatus::Cancelled.database_value(),
            ],
        )?;
        transaction.commit()?;
        Ok(updated > 0)
    })
    .map_err(engine_repository_error_from_database)
}

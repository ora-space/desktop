use super::engine_repository_error_from_database;
use crate::repository::RepositoryPool;
use ora_application::RepositoryError;
use ora_domain::{WorkflowRunId, WorkflowRunStatus, WorkflowSnapshotId};
use rusqlite::{Transaction, TransactionBehavior, params};

/// Points a Failed/Cancelled run at another snapshot, stores the migrated payload and, for a run
/// with run-local debug revisions, starts a new configuration baseline in the same transaction.
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
        // A switch starts a new configuration baseline; it must not overtake a confirmation that
        // is still stopping or holds an unreleased fence.
        crate::workflow_debug::fence::ensure_inactive(&transaction, run_id.as_ref())?;
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
        // The switch is the only event that resets the configuration baseline. It commits the
        // new snapshot together with a new run revision, so earlier run-local edits stay
        // readable as history and are never replayed again.
        if updated > 0 {
            transaction.execute(
                "INSERT INTO workflow_debug_baselines(run_id,revision,snapshot_id)
                 SELECT ?1,COALESCE((SELECT MAX(revision) FROM workflow_debug_revisions WHERE run_id=?1),0)+1,?2
                 WHERE EXISTS(SELECT 1 FROM workflow_debug_revisions WHERE run_id=?1)",
                params![run_id.as_ref(), snapshot_id.as_ref()],
            )?;
        }
        transaction.commit()?;
        Ok(updated > 0)
    })
    .map_err(engine_repository_error_from_database)
}

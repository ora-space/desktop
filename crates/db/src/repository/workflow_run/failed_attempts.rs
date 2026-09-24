//! Earlier failed attempts of a run's nodes, for the run inspector's attempt history.

use super::map_node_run_row;
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId};
use rusqlite::params;

/// Lists the soft-deleted `Failed` node runs of one run, oldest first.
///
/// Automatic retries, resume, and restart all clear a failed attempt by soft-deleting it, so
/// these rows are exactly the attempts that failed and were run again (or whose run was
/// restarted); each keeps its `payload.error_detail`.
pub(super) fn list_failed_attempts(
    connection: &rusqlite::Connection,
    run_id: &WorkflowRunId,
) -> Result<Vec<WorkflowNodeRun>, crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, run_id, scope_id, node_id, node_type, session_id, status, input, output, error, payload, iteration,
                started_at, finished_at, created_at, updated_at, is_deleted
         FROM workflow_node_runs
         WHERE run_id = ?1 AND is_deleted = 1 AND status = ?2
         ORDER BY created_at, id",
    )?;
    let mut rows = statement.query(params![
        run_id.as_ref(),
        WorkflowNodeStatus::Failed.database_value()
    ])?;
    let mut attempts = Vec::new();
    while let Some(row) = rows.next()? {
        attempts.push(map_node_run_row(row)?);
    }
    Ok(attempts)
}

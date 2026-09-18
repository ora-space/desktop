use super::current_nodes::current_nodes_to_state;
use super::engine_repository_error_from_database;
use super::reset_run_execution_state;
use crate::repository::RepositoryPool;
use ora_application::{RepositoryError, RestartWorkflowRunResult};
use ora_domain::{WorkflowRunId, WorkflowRunStatus};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

/// Restarts a non-running run: soft-deletes its node runs and resets it to `Pending`.
pub(super) fn restart_run(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    now: i64,
) -> Result<RestartWorkflowRunResult, RepositoryError> {
    pool.with_connection_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let state = transaction
            .query_row(
                "SELECT run_status, payload FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                params![run_id.as_ref()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        let Some((status, payload)) = state else {
            return Ok(RestartWorkflowRunResult::NotFound);
        };
        if WorkflowRunStatus::from_database_value(status)? == WorkflowRunStatus::Running {
            return Ok(RestartWorkflowRunResult::NotRestartable);
        }
        // A restart is a fresh execution: the previous node runs are soft-deleted so their
        // history stays queryable, while the fresh run starts from an empty node-run set.
        transaction.execute(
            "UPDATE workflow_node_runs SET is_deleted = 1, updated_at = ?2
             WHERE run_id = ?1 AND is_deleted = 0",
            params![run_id.as_ref(), now],
        )?;
        let state = current_nodes_to_state(&[])?;
        transaction.execute(
            "UPDATE workflow_runs SET run_status = ?2, state = ?3, output = NULL, error = NULL, started_at = NULL, finished_at = NULL, updated_at = ?4
             WHERE id = ?1 AND is_deleted = 0",
            params![
                run_id.as_ref(),
                WorkflowRunStatus::Pending.database_value(),
                state,
                now,
            ],
        )?;
        // Reset computed values while preserving the separately stored run instruction and
        // the deployment values owned by the Start node.
        reset_run_execution_state(&transaction, run_id, payload.as_deref())?;
        crate::repository::workflow_scope::restart_root_scope(&transaction, run_id, now)?;
        transaction.commit()?;
        Ok(RestartWorkflowRunResult::Restarted)
    })
    .map_err(engine_repository_error_from_database)
}

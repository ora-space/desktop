use super::current_nodes::current_nodes_to_state;
use super::engine_repository_error_from_database;
use crate::repository::RepositoryPool;
use ora_application::{CancelWorkflowRunResult, RepositoryError};
use ora_domain::{WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus, WorkflowScopeStatus};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

/// Cancels a running run: the run and its non-terminal node runs become `Cancelled`.
pub(super) fn cancel_run(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    now: i64,
) -> Result<CancelWorkflowRunResult, RepositoryError> {
    pool.with_connection_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let status = transaction
            .query_row(
                "SELECT run_status FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                params![run_id.as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(status) = status else {
            return Ok(CancelWorkflowRunResult::NotFound);
        };
        if WorkflowRunStatus::from_database_value(status)? != WorkflowRunStatus::Running {
            return Ok(CancelWorkflowRunResult::NotActive);
        }
        transaction.execute(
            "UPDATE workflow_node_runs SET status = ?2, finished_at = ?3, updated_at = ?3
             WHERE run_id = ?1 AND status IN (0, 1) AND is_deleted = 0",
            params![
                run_id.as_ref(),
                WorkflowNodeStatus::Cancelled.database_value(),
                now
            ],
        )?;
        let state = current_nodes_to_state(&[])?;
        transaction.execute(
            "UPDATE workflow_runs SET run_status = ?2, finished_at = ?3, updated_at = ?3, state = ?4
             WHERE id = ?1 AND is_deleted = 0",
            params![
                run_id.as_ref(),
                WorkflowRunStatus::Cancelled.database_value(),
                now,
                state,
            ],
        )?;
        transaction.execute(
            "UPDATE workflow_execution_scopes SET status = ?2, updated_at = ?3
             WHERE run_id = ?1 AND parent_loop_node_run_id IS NOT NULL
               AND status IN (0, 1)",
            params![
                run_id.as_ref(),
                WorkflowScopeStatus::Cancelled.database_value(),
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(CancelWorkflowRunResult::Cancelled)
    })
    .map_err(engine_repository_error_from_database)
}

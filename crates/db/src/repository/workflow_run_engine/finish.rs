use super::current_nodes::current_nodes_to_state;
use super::engine_repository_error_from_database;
use crate::repository::RepositoryPool;
use ora_application::RepositoryError;
use ora_domain::{WorkflowRunId, WorkflowRunStatus};
use rusqlite::{Transaction, TransactionBehavior, params};

/// Finishes a run as succeeded with the given output.
pub(super) fn finish_run(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    output: Option<String>,
    now: i64,
) -> Result<(), RepositoryError> {
    pool.with_connection_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let state = current_nodes_to_state(&[])?;
        transaction.execute(
            "UPDATE workflow_runs SET run_status = ?2, output = ?3, finished_at = ?4, updated_at = ?4, state = ?5
             WHERE id = ?1 AND is_deleted = 0 AND run_status IN (0, 1)",
            params![
                run_id.as_ref(),
                WorkflowRunStatus::Succeeded.database_value(),
                output,
                now,
                state,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    })
    .map_err(engine_repository_error_from_database)
}

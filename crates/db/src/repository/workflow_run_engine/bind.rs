use super::engine_repository_error_from_database;
use crate::repository::RepositoryPool;
use ora_application::{BindWorkflowNodeSessionResult, RepositoryError};
use ora_domain::{SessionId, WorkflowNodeRunId, WorkflowNodeStatus};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

/// Binds a prepared session to an in-flight node run.
///
/// Design rule D2: a run that is already `Failed` still accepts bindings for nodes whose own
/// status is `Running`, so those in-flight siblings persist `Succeeded` or `Failed` on their own
/// merits. Cancellation still rejects here because `cancel_run` marks every non-terminal node
/// `Cancelled` in the same transaction that cancels the run.
pub(super) fn bind_node_run_session(
    pool: &RepositoryPool,
    node_run_id: &WorkflowNodeRunId,
    session_id: &SessionId,
    now: i64,
) -> Result<BindWorkflowNodeSessionResult, RepositoryError> {
    pool.with_connection_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let state = transaction
            .query_row(
                "SELECT nr.status,
                        EXISTS(
                            SELECT 1 FROM sessions s
                            WHERE s.id = ?2
                              AND s.workspace_id = wr.workspace_id
                              AND s.is_deleted = 0
                        )
                 FROM workflow_node_runs nr
                 JOIN workflow_runs wr ON wr.id = nr.run_id
                 WHERE nr.id = ?1 AND nr.is_deleted = 0 AND wr.is_deleted = 0",
                params![node_run_id.as_ref(), session_id.as_ref()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((node_status, session_matches_workspace)) = state else {
            return Ok(BindWorkflowNodeSessionResult::NotFound);
        };
        if session_matches_workspace == 0 {
            return Ok(BindWorkflowNodeSessionResult::NotFound);
        }
        if WorkflowNodeStatus::from_database_value(node_status)? != WorkflowNodeStatus::Running {
            return Ok(BindWorkflowNodeSessionResult::NotRunning);
        }
        transaction.execute(
            "UPDATE workflow_node_runs SET session_id = ?2, updated_at = ?3
             WHERE id = ?1 AND is_deleted = 0",
            params![node_run_id.as_ref(), session_id.as_ref(), now],
        )?;
        transaction.commit()?;
        Ok(BindWorkflowNodeSessionResult::Bound)
    })
    .map_err(engine_repository_error_from_database)
}

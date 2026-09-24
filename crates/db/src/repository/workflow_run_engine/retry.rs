//! Automatic retry transactions: replacing a failed attempt by a waiting attempt, starting it
//! when its backoff elapses, and settling waits the run can no longer honour.

use super::failure_detail::{deleted_attempt_count, persist_failed_node_run};
use super::{SqliteWorkflowRunEngineRepository, engine_repository_error_from_database};
use ora_application::{
    AUTO_RETRY_KEY, BeginNodeRetryResult, NodeAutoRetry, NodeFailure, NodeRetryToSchedule,
    NodeRetryWait, RETRY_CHAIN_KEY, RETRY_WAIT_KEY, RepositoryError, ScheduleNodeRetryResult,
    WorkflowRetryRepository, retry_chain_from_payload,
};
use ora_domain::{WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunStatus};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Map, Value};

/// SQLite JSON path of the waiting marker (`payload.retry_wait`).
pub(super) const RETRY_WAIT_PATH: &str = "$.retry_wait";

/// Error recorded on a waiting attempt the run gave up on before it could start.
pub(super) const RETRY_ABANDONED: &str = r#"{"reason":"retry_abandoned"}"#;

impl WorkflowRetryRepository for SqliteWorkflowRunEngineRepository {
    fn schedule_node_retry(
        &self,
        failed_node_run_id: &WorkflowNodeRunId,
        failure: &NodeFailure,
        retry: &NodeRetryToSchedule,
        now: i64,
    ) -> Result<ScheduleNodeRetryResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((run_id, node_id, node_type, status, scope_id, input, iteration, payload, run_status)) = transaction
                    .query_row(
                        "SELECT nr.run_id, nr.node_id, nr.node_type, nr.status, nr.scope_id, nr.input,
                                nr.iteration, nr.payload, wr.run_status
                         FROM workflow_node_runs nr
                         JOIN workflow_runs wr ON wr.id = nr.run_id
                         WHERE nr.id = ?1 AND nr.is_deleted = 0 AND wr.is_deleted = 0",
                        params![failed_node_run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, Option<String>>(5)?,
                                row.get::<_, Option<u32>>(6)?,
                                row.get::<_, Option<String>>(7)?,
                                row.get::<_, i64>(8)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Ok(ScheduleNodeRetryResult::NotFound);
                };
                if WorkflowNodeStatus::from_database_value(status)? != WorkflowNodeStatus::Running {
                    return Ok(ScheduleNodeRetryResult::NotRunning);
                }
                if WorkflowRunStatus::from_database_value(run_status)? != WorkflowRunStatus::Running {
                    return Ok(ScheduleNodeRetryResult::RunNotActive);
                }
                // The failed attempt keeps its full failure record and leaves the live set the
                // same way a resume clears it, so attempt numbering and the previous-failure
                // lookup treat both paths alike.
                persist_failed_node_run(
                    &transaction,
                    failed_node_run_id.as_ref(),
                    &run_id,
                    &node_id,
                    failure,
                    payload.as_deref(),
                    now,
                )?;
                transaction.execute(
                    "UPDATE workflow_node_runs SET is_deleted = 1, updated_at = ?2 WHERE id = ?1",
                    params![failed_node_run_id.as_ref(), now],
                )?;
                let attempt =
                    deleted_attempt_count(&transaction, &run_id, &node_id, iteration, &scope_id)?
                        .saturating_add(1);
                let wait = NodeRetryWait {
                    attempt,
                    max_attempt: attempt
                        .saturating_add(retry.max_retries.saturating_sub(retry.retry)),
                    retry: retry.retry,
                    max_retries: retry.max_retries,
                    delay_ms: retry.delay_ms,
                    scheduled_at: now,
                    due_at: retry.due_at,
                    previous_node_run_id: failed_node_run_id.to_string(),
                };
                let marker = NodeAutoRetry {
                    retry: retry.retry,
                    max_retries: retry.max_retries,
                };
                // The chain links every attempt since the last manual action, so a resume after
                // exhaustion can roll back all of them as one unit.
                let mut chain = retry_chain_from_payload(payload.as_deref());
                chain.push(failed_node_run_id.to_string());
                let payload = Value::Object(Map::from_iter([
                    (RETRY_WAIT_KEY.to_string(), serde_json::to_value(&wait)?),
                    (AUTO_RETRY_KEY.to_string(), serde_json::to_value(marker)?),
                    (RETRY_CHAIN_KEY.to_string(), serde_json::to_value(chain)?),
                ]));
                // `started_at` stays NULL until the wait elapses; the row is `Running` so every
                // scheduling, cancel, and boot-sweep rule already treats it as in flight.
                transaction.execute(
                    "INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, session_id, status, input, output, error, payload, iteration, started_at, finished_at, created_at, updated_at, is_deleted, scope_id)
                     VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, NULL, NULL, ?7, ?8, NULL, NULL, ?9, ?9, 0, ?10)",
                    params![
                        retry.node_run_id.as_ref(),
                        &run_id,
                        &node_id,
                        &node_type,
                        WorkflowNodeStatus::Running.database_value(),
                        input,
                        payload.to_string(),
                        iteration,
                        now,
                        &scope_id,
                    ],
                )?;
                transaction.commit()?;
                Ok(ScheduleNodeRetryResult::Scheduled)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn begin_node_retry(
        &self,
        node_run_id: &WorkflowNodeRunId,
        now: i64,
    ) -> Result<BeginNodeRetryResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((status, payload, run_status)) = transaction
                    .query_row(
                        "SELECT nr.status, nr.payload, wr.run_status
                         FROM workflow_node_runs nr
                         JOIN workflow_runs wr ON wr.id = nr.run_id
                         WHERE nr.id = ?1 AND nr.is_deleted = 0 AND wr.is_deleted = 0",
                        params![node_run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Ok(BeginNodeRetryResult::NotFound);
                };
                let mut payload = match payload
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                {
                    Some(Value::Object(payload)) => payload,
                    _ => return Ok(BeginNodeRetryResult::NotWaiting),
                };
                let Some(wait) = payload
                    .remove(RETRY_WAIT_KEY)
                    .and_then(|wait| serde_json::from_value::<NodeRetryWait>(wait).ok())
                else {
                    return Ok(BeginNodeRetryResult::NotWaiting);
                };
                if WorkflowNodeStatus::from_database_value(status)? != WorkflowNodeStatus::Running {
                    return Ok(BeginNodeRetryResult::NotWaiting);
                }
                if WorkflowRunStatus::from_database_value(run_status)? != WorkflowRunStatus::Running
                {
                    // Every path that ends a run settles its waits; this is the safety net for a
                    // run that left `Running` some other way.
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?2, error = ?3, payload = ?4,
                                finished_at = ?5, updated_at = ?5
                         WHERE id = ?1",
                        params![
                            node_run_id.as_ref(),
                            WorkflowNodeStatus::Cancelled.database_value(),
                            RETRY_ABANDONED,
                            Value::Object(payload).to_string(),
                            now,
                        ],
                    )?;
                    transaction.commit()?;
                    return Ok(BeginNodeRetryResult::Abandoned);
                }
                if now < wait.due_at {
                    return Ok(BeginNodeRetryResult::NotDue {
                        due_at: wait.due_at,
                    });
                }
                transaction.execute(
                    "UPDATE workflow_node_runs SET payload = ?2, started_at = ?3, updated_at = ?3
                     WHERE id = ?1",
                    params![
                        node_run_id.as_ref(),
                        Value::Object(payload).to_string(),
                        now
                    ],
                )?;
                let started =
                    super::super::workflow_run::find_node_run_by_id(&transaction, node_run_id)?
                        .ok_or(crate::DatabaseError::IncompleteWorkflowRunContext)?;
                transaction.commit()?;
                Ok(BeginNodeRetryResult::Started(Box::new(started)))
            })
            .map_err(engine_repository_error_from_database)
    }
}

/// Settles every waiting attempt of one run as `status`, inside a larger transaction.
///
/// Called wherever a run leaves `Running`, so a pending retry never fires into a terminal run
/// and a failed run is immediately resumable. `error` is kept when `None` (cancel records none).
pub(super) fn settle_retry_waits(
    transaction: &Transaction<'_>,
    run_id: &str,
    status: WorkflowNodeStatus,
    error: Option<&str>,
    now: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE workflow_node_runs
         SET status = ?2, error = COALESCE(?3, error), payload = json_remove(payload, ?4),
             finished_at = ?5, updated_at = ?5
         WHERE run_id = ?1 AND status = ?6 AND is_deleted = 0
           AND json_extract(CASE WHEN json_valid(payload) THEN payload END, ?4) IS NOT NULL",
        params![
            run_id,
            status.database_value(),
            error,
            RETRY_WAIT_PATH,
            now,
            WorkflowNodeStatus::Running.database_value(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// The SQL path and the payload key the application writes must name the same member.
    #[test]
    fn retry_wait_path_addresses_the_application_payload_key() {
        assert_eq!(RETRY_WAIT_PATH, format!("$.{RETRY_WAIT_KEY}"));
    }
}

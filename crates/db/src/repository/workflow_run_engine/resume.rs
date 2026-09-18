use super::current_nodes::current_nodes_to_state;
use super::engine_repository_error_from_database;
use crate::repository::RepositoryPool;
use ora_application::{
    RepositoryError, ResumeWorkflowRunResult, WorkflowRunPayload, running_row_blocks_resume,
};
use ora_domain::{WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus, WorkflowScopeStatus};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter};
use std::collections::BTreeSet;

/// Soft-deletes the listed node runs and their pool writes, then returns the run to `Running`.
pub(super) fn resume_from_failure(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    node_ids_to_clear: &[String],
    now: i64,
) -> Result<ResumeWorkflowRunResult, RepositoryError> {
    pool.with_connection_mut(|connection| {
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let run = transaction
            .query_row(
                "SELECT run_status, payload FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                params![run_id.as_ref()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((status, serialized_payload)) = run else {
            return Ok(ResumeWorkflowRunResult::NotFound);
        };
        if !matches!(
            WorkflowRunStatus::from_database_value(status)?,
            WorkflowRunStatus::Failed | WorkflowRunStatus::Cancelled
        ) || node_ids_to_clear.is_empty()
        {
            return Ok(ResumeWorkflowRunResult::NotResumable);
        }
        // Container rows parked by the terminal run do not count as live work; see
        // `running_row_blocks_resume`.
        let has_running_node = transaction
            .prepare(
                "SELECT node_type FROM workflow_node_runs
                 WHERE run_id = ?1 AND status = ?2 AND is_deleted = 0",
            )?
            .query_map(
                params![
                    run_id.as_ref(),
                    WorkflowNodeStatus::Running.database_value()
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|node_type| running_row_blocks_resume(node_type));
        if has_running_node {
            return Ok(ResumeWorkflowRunResult::NotResumable);
        }

        let placeholders = std::iter::repeat_n("?", node_ids_to_clear.len())
            .collect::<Vec<_>>()
            .join(", ");
        // A cleared Loop owner takes its rounds with it: the round scopes close and their
        // child rows are soft-deleted, so the rerun starts from round 1 with no active round.
        let owners_sql = format!(
            "SELECT id FROM workflow_node_runs WHERE run_id = ? AND is_deleted = 0 AND node_id IN ({placeholders})"
        );
        let mut owner_parameters =
            Vec::<rusqlite::types::Value>::with_capacity(node_ids_to_clear.len() + 1);
        owner_parameters.push(run_id.as_ref().to_string().into());
        owner_parameters.extend(node_ids_to_clear.iter().cloned().map(Into::into));
        transaction.execute(
            &format!(
                "UPDATE workflow_node_runs SET is_deleted = 1, updated_at = ?
                 WHERE is_deleted = 0 AND scope_id IN (
                     SELECT id FROM workflow_execution_scopes
                     WHERE parent_loop_node_run_id IN ({owners_sql}))"
            ),
            params_from_iter(
                std::iter::once(rusqlite::types::Value::from(now)).chain(owner_parameters.iter().cloned()),
            ),
        )?;
        transaction.execute(
            &format!(
                "UPDATE workflow_execution_scopes SET status = ?, updated_at = ?
                 WHERE status IN (0, 1) AND parent_loop_node_run_id IN ({owners_sql})"
            ),
            params_from_iter(
                [
                    rusqlite::types::Value::from(WorkflowScopeStatus::Cancelled.database_value()),
                    rusqlite::types::Value::from(now),
                ]
                .into_iter()
                .chain(owner_parameters),
            ),
        )?;
        let sql = format!(
            "UPDATE workflow_node_runs SET is_deleted = 1, updated_at = ?
                     WHERE run_id = ? AND is_deleted = 0 AND node_id IN ({placeholders})"
        );
        let mut parameters = Vec::<rusqlite::types::Value>::with_capacity(node_ids_to_clear.len() + 2);
        parameters.push(now.into());
        parameters.push(run_id.as_ref().to_string().into());
        parameters.extend(node_ids_to_clear.iter().cloned().map(Into::into));
        transaction.execute(&sql, params_from_iter(parameters))?;

        if let Some(serialized_payload) = serialized_payload {
            let cleared_writers = node_ids_to_clear
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let mut payload: WorkflowRunPayload = serde_json::from_str(&serialized_payload)?;
            let pool = &mut payload.variable_pool;
            pool.values.retain(|selector, _| {
                pool.catalog
                    .get(selector)
                    .is_none_or(|definition| !cleared_writers.contains(definition.writer.as_str()))
            });
            pool.revision = pool.revision.saturating_add(1);
            payload
                .condition_decisions
                .retain(|node_id, _| !cleared_writers.contains(node_id.as_str()));
            payload
                .iteration_ledger
                .retain(|owner, _| !cleared_writers.contains(owner.as_str()));
            payload.iteration_condition_decisions.retain(|key, _| {
                let node_id = key.split('#').next().unwrap_or(key);
                !cleared_writers.contains(node_id)
            });
            transaction.execute(
                "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1 AND is_deleted = 0",
                params![run_id.as_ref(), serde_json::to_string(&payload)?],
            )?;
        }

        let state = current_nodes_to_state(&[])?;
        transaction.execute(
            "UPDATE workflow_runs SET run_status = ?2, error = NULL, finished_at = NULL, state = ?3, updated_at = ?4
                     WHERE id = ?1 AND is_deleted = 0",
            params![
                run_id.as_ref(),
                WorkflowRunStatus::Running.database_value(),
                state,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(ResumeWorkflowRunResult::Resumed)
    })
    .map_err(engine_repository_error_from_database)
}

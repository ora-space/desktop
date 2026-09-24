//! Node completion and failure transactions shared by root and Loop scopes.

use super::failure_detail::persist_failed_node_run;
use super::iteration::write_pool_variable;
use super::payload_json::merge_complete_payload;
use super::retry::{RETRY_ABANDONED, settle_retry_waits};
use super::*;

/// Completes one node and writes its outputs to the owning execution scope.
pub(super) fn complete(
    repository: &SqliteWorkflowRunEngineRepository,
    node_run_id: &WorkflowNodeRunId,
    output: Option<String>,
    structured_output: Option<serde_json::Value>,
    stop_reason: Option<String>,
    file_changes: Vec<FileChange>,
    now: i64,
) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
    repository
        .pool
        .with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            let Some((run_id, node_id, node_type, status, run_payload, scope_id, root_scope_id, scope_state, iteration, node_payload)) = transaction
                .query_row(
                    "SELECT nr.run_id, nr.node_id, nr.node_type, nr.status, wr.payload,
                            nr.scope_id, root.scope_id, scope.state, nr.iteration, nr.payload
                     FROM workflow_node_runs nr
                     JOIN workflow_runs wr ON wr.id = nr.run_id
                     JOIN workflow_run_root_scopes root ON root.run_id = nr.run_id
                     JOIN workflow_execution_scopes scope ON scope.id = nr.scope_id
                     WHERE nr.id = ?1 AND nr.is_deleted = 0 AND wr.is_deleted = 0",
                    params![node_run_id.as_ref()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, Option<u32>>(8)?,
                            row.get::<_, Option<String>>(9)?,
                        ))
                    },
                )
                .optional()?
            else {
                return Ok(AdvanceWorkflowRunResult::NotFound);
            };
            // Pending interactive nodes complete through the same callback path as running nodes.
            if !matches!(
                WorkflowNodeStatus::from_database_value(status)?,
                WorkflowNodeStatus::Running | WorkflowNodeStatus::Pending
            ) {
                return Ok(AdvanceWorkflowRunResult::NotRunning);
            }
            // Merge onto the existing payload so a success never drops the checkpoint keys.
            let payload = merge_complete_payload(node_payload, stop_reason, file_changes)?;
            if scope_id == root_scope_id {
                update_run_execution_state(
                    &transaction,
                    &run_id,
                    &node_id,
                    &node_type,
                    iteration,
                    output.as_deref(),
                    structured_output.as_ref(),
                    run_payload.as_deref(),
                )?;
            } else {
                update_scope_execution_state(
                    &transaction,
                    &scope_id,
                    scope_state.as_deref(),
                    ScopeNodeCompletion {
                        node_id: &node_id,
                        node_type: &node_type,
                        output: output.as_deref(),
                        structured_output: structured_output.as_ref(),
                    },
                    now,
                )?;
            }
            // Condition decisions are scheduler state and must not become public node output.
            let persisted_output = (node_type != "condition").then_some(output).flatten();
            transaction.execute(
                "UPDATE workflow_node_runs SET status = ?2, output = ?3, payload = ?4, finished_at = ?5, updated_at = ?5
                 WHERE id = ?1 AND is_deleted = 0",
                params![
                    node_run_id.as_ref(),
                    WorkflowNodeStatus::Succeeded.database_value(),
                    persisted_output,
                    payload,
                    now,
                ],
            )?;
            if scope_id == root_scope_id {
                let run_id = WorkflowRunId::new(run_id);
                rewrite_current_nodes(&transaction, &run_id, now, |current_nodes| {
                    current_nodes.retain(|id| id != &node_id);
                })?;
            }
            transaction.commit()?;
            Ok(AdvanceWorkflowRunResult::Advanced)
        })
        .map_err(engine_repository_error_from_database)
}

/// Fails one node and propagates the failure to its owner.
///
/// Root-scope nodes follow design rule D2: only the failed row and the run change state, and
/// in-flight siblings finish on their own merits. Inside a Loop round the round is isolated,
/// so its active siblings are cancelled and the failure climbs to the parent Loop node.
pub(super) fn fail(
    repository: &SqliteWorkflowRunEngineRepository,
    node_run_id: &WorkflowNodeRunId,
    failure: NodeFailure,
    propagation: FailurePropagation,
    now: i64,
) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
    repository
        .pool
        .with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            let Some((run_id, node_id, status, scope_id, root_scope_id, parent_loop_id, parent_node_id, node_payload)) = transaction
                .query_row(
                    "SELECT node.run_id, node.node_id, node.status, node.scope_id, root.scope_id,
                            scope.parent_loop_node_run_id, parent.node_id, node.payload
                     FROM workflow_node_runs node
                     JOIN workflow_run_root_scopes root ON root.run_id = node.run_id
                     JOIN workflow_execution_scopes scope ON scope.id = node.scope_id
                     LEFT JOIN workflow_node_runs parent ON parent.id = scope.parent_loop_node_run_id
                     WHERE node.id = ?1 AND node.is_deleted = 0",
                    params![node_run_id.as_ref()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                        ))
                    },
                )
                .optional()?
            else {
                return Ok(AdvanceWorkflowRunResult::NotFound);
            };
            if WorkflowNodeStatus::from_database_value(status)? != WorkflowNodeStatus::Running {
                return Ok(AdvanceWorkflowRunResult::NotRunning);
            }
            persist_failed_node_run(
                &transaction,
                node_run_id.as_ref(),
                &run_id,
                &node_id,
                &failure,
                node_payload.as_deref(),
                now,
            )?;
            let error = failure.message;
            if propagation == FailurePropagation::Composite {
                rewrite_current_nodes(&transaction, &WorkflowRunId::new(run_id), now, |nodes| {
                    nodes.retain(|id| id != &node_id);
                })?;
                transaction.commit()?;
                return Ok(AdvanceWorkflowRunResult::Advanced);
            }
            // The run fails now, so no pending retry of any node may fire into it.
            settle_retry_waits(
                &transaction,
                &run_id,
                WorkflowNodeStatus::Cancelled,
                Some(RETRY_ABANDONED),
                now,
            )?;
            if scope_id != root_scope_id {
                transaction.execute(
                    "UPDATE workflow_node_runs SET status = ?2,
                            error = COALESCE(error, '{\"reason\":\"sibling_failed\"}'),
                            finished_at = COALESCE(finished_at, ?3), updated_at = ?3
                     WHERE scope_id = ?1 AND id != ?4 AND status IN (0, 1) AND is_deleted = 0",
                    params![
                        &scope_id,
                        WorkflowNodeStatus::Cancelled.database_value(),
                        now,
                        node_run_id.as_ref(),
                    ],
                )?;
                transaction.execute(
                    "UPDATE workflow_execution_scopes SET status = ?2, updated_at = ?3
                     WHERE id = ?1 AND status IN (0, 1)",
                    params![
                        &scope_id,
                        WorkflowScopeStatus::Failed.database_value(),
                        now,
                    ],
                )?;
                if let Some(parent_loop_id) = parent_loop_id.as_deref() {
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?2, error = ?3,
                                finished_at = ?4, updated_at = ?4
                         WHERE id = ?1 AND status IN (0, 1) AND is_deleted = 0",
                        params![
                            parent_loop_id,
                            WorkflowNodeStatus::Failed.database_value(),
                            &error,
                            now,
                        ],
                    )?;
                }
            }
            let run_id = WorkflowRunId::new(run_id);
            let anchor = parent_node_id.unwrap_or(node_id);
            rewrite_current_nodes(&transaction, &run_id, now, |current_nodes| {
                current_nodes.clear();
                current_nodes.push(anchor.clone());
            })?;
            // D2: only promote the run while it is still Running so a late sibling failure
            // cannot overwrite an already-terminal run.
            transaction.execute(
                "UPDATE workflow_runs SET run_status = ?2, error = ?3, finished_at = ?4, updated_at = ?4
                 WHERE id = ?1 AND is_deleted = 0 AND run_status = ?5",
                params![
                    run_id.as_ref(),
                    WorkflowRunStatus::Failed.database_value(),
                    error,
                    now,
                    WorkflowRunStatus::Running.database_value(),
                ],
            )?;
            transaction.commit()?;
            Ok(AdvanceWorkflowRunResult::Advanced)
        })
        .map_err(engine_repository_error_from_database)
}

/// The child result fields consumed while updating one round's isolated state.
struct ScopeNodeCompletion<'a> {
    node_id: &'a str,
    node_type: &'a str,
    output: Option<&'a str>,
    structured_output: Option<&'a serde_json::Value>,
}

/// Commits child outputs and branch decisions to the owning round.
fn update_scope_execution_state(
    transaction: &Transaction<'_>,
    scope_id: &str,
    serialized_state: Option<&str>,
    completion: ScopeNodeCompletion<'_>,
    now: i64,
) -> Result<(), crate::DatabaseError> {
    let Some(serialized_state) = serialized_state else {
        return Err(crate::DatabaseError::IncompleteWorkflowRunContext);
    };
    let mut state: LoopRoundExecutionState = serde_json::from_str(serialized_state)?;
    let mut changed = if completion.node_type != "condition"
        && let Some(output) = completion.output
    {
        write_pool_variable(
            &mut state.variable_pool,
            &format!("{}.output", completion.node_id),
            completion.node_id,
            serde_json::Value::String(output.to_string()),
        )?
    } else {
        false
    };
    match completion.node_type {
        "start" => {}
        "condition" => {
            if let Some(output) = completion.output {
                changed |= state
                    .condition_decisions
                    .get(completion.node_id)
                    .map(String::as_str)
                    != Some(output);
                state
                    .condition_decisions
                    .insert(completion.node_id.to_string(), output.to_string());
            }
        }
        "agent" => {
            if let Some(structured) = completion.structured_output {
                changed |= write_pool_variable(
                    &mut state.variable_pool,
                    &format!("{}.structured_output", completion.node_id),
                    completion.node_id,
                    structured.clone(),
                )?;
            }
        }
        _ => {}
    }
    if changed {
        transaction.execute(
            "UPDATE workflow_execution_scopes SET state = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 1",
            params![scope_id, serde_json::to_string(&state)?, now],
        )?;
    }
    Ok(())
}

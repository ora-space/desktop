//! Atomic persistence operations for isolated Loop rounds.

use super::iteration::write_pool_variable;
use super::payload_json::merge_complete_payload;
use super::*;

/// Creates a round scope and its Start node in one transaction.
pub(super) fn start(
    repository: &SqliteWorkflowRunEngineRepository,
    run_id: &WorkflowRunId,
    round: &LoopRoundToStart,
    now: i64,
) -> Result<(), RepositoryError> {
    repository
        .pool
        .with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            if round.start_node_run.scope_id != round.id {
                return Err(crate::DatabaseError::IncompleteWorkflowRunContext);
            }
            transaction.execute(
                "INSERT INTO workflow_execution_scopes
                 (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    round.id.as_ref(),
                    run_id.as_ref(),
                    round.parent_loop_node_run_id.as_ref(),
                    round.round_index,
                    WorkflowScopeStatus::Running.database_value(),
                    &round.state,
                    now,
                ],
            )?;
            insert_node_run(&transaction, run_id, &round.start_node_run, now)?;
            transaction.commit()?;
            Ok(())
        })
        .map_err(engine_repository_error_from_database)
}

/// Starts every newly ready child in the specified active round.
pub(super) fn start_ready_nodes(
    repository: &SqliteWorkflowRunEngineRepository,
    scope_id: &WorkflowScopeId,
    node_runs: &[NodeRunToStart],
    now: i64,
) -> Result<(), RepositoryError> {
    repository
        .pool
        .with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            let run_id = transaction.query_row(
                "SELECT run_id FROM workflow_execution_scopes WHERE id = ?1 AND status = 1",
                params![scope_id.as_ref()],
                |row| row.get::<_, String>(0),
            )?;
            let run_id = WorkflowRunId::new(run_id);
            for node_run in node_runs {
                if node_run.scope_id != *scope_id {
                    return Err(crate::DatabaseError::IncompleteWorkflowRunContext);
                }
                insert_node_run(&transaction, &run_id, node_run, now)?;
            }
            transaction.commit()?;
            Ok(())
        })
        .map_err(engine_repository_error_from_database)
}

/// Settles one round and atomically continues, succeeds, or fails its parent Loop.
pub(super) fn advance(
    repository: &SqliteWorkflowRunEngineRepository,
    scope_id: &WorkflowScopeId,
    advance: &LoopRoundAdvance,
    now: i64,
) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
    repository
        .pool
        .with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            let round = transaction
                .query_row(
                    "SELECT scope.run_id, scope.parent_loop_node_run_id, parent.node_id,
                            scope.status, run.payload, parent.status, run.run_status,
                            parent.payload
                     FROM workflow_execution_scopes scope
                     JOIN workflow_node_runs parent ON parent.id = scope.parent_loop_node_run_id
                     JOIN workflow_runs run ON run.id = scope.run_id
                     WHERE scope.id = ?1 AND parent.is_deleted = 0 AND run.is_deleted = 0",
                    params![scope_id.as_ref()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, Option<String>>(7)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                run_id,
                parent_run_id,
                parent_node_id,
                status,
                run_payload,
                parent_status,
                run_status,
                parent_payload,
            )) = round
            else {
                return Ok(AdvanceWorkflowRunResult::NotFound);
            };
            if WorkflowScopeStatus::from_database_value(status)? != WorkflowScopeStatus::Running
                || WorkflowNodeStatus::from_database_value(parent_status)?
                    != WorkflowNodeStatus::Running
                || WorkflowRunStatus::from_database_value(run_status)? != WorkflowRunStatus::Running
            {
                return Ok(AdvanceWorkflowRunResult::NotRunning);
            }
            let active_children = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM workflow_node_runs
                    WHERE scope_id = ?1 AND status IN (0, 1) AND is_deleted = 0
                )",
                params![scope_id.as_ref()],
                |row| row.get::<_, bool>(0),
            )?;
            if active_children && !matches!(advance, LoopRoundAdvance::Fail { .. }) {
                return Ok(AdvanceWorkflowRunResult::NotRunning);
            }

            match advance {
                LoopRoundAdvance::Continue { next } => {
                    if next.parent_loop_node_run_id.as_ref() != parent_run_id
                        || next.start_node_run.scope_id != next.id
                    {
                        return Err(crate::DatabaseError::IncompleteWorkflowRunContext);
                    }
                    settle(&transaction, scope_id, WorkflowScopeStatus::Succeeded, now)?;
                    transaction.execute(
                        "INSERT INTO workflow_execution_scopes
                         (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![
                            next.id.as_ref(),
                            &run_id,
                            next.parent_loop_node_run_id.as_ref(),
                            next.round_index,
                            WorkflowScopeStatus::Running.database_value(),
                            &next.state,
                            now,
                        ],
                    )?;
                    insert_node_run(
                        &transaction,
                        &WorkflowRunId::new(run_id),
                        &next.start_node_run,
                        now,
                    )?;
                }
                LoopRoundAdvance::Succeed { outputs } => {
                    settle(&transaction, scope_id, WorkflowScopeStatus::Succeeded, now)?;
                    let mut payload = run_payload
                        .as_deref()
                        .map(serde_json::from_str::<WorkflowRunPayload>)
                        .transpose()?
                        .ok_or(crate::DatabaseError::IncompleteWorkflowRunContext)?;
                    for (name, value) in outputs {
                        write_pool_variable(
                            &mut payload.variable_pool,
                            &format!("{parent_node_id}.{name}"),
                            &parent_node_id,
                            value.clone(),
                        )?;
                    }
                    let serialized_output = serde_json::to_string(outputs)?;
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?2, output = ?3, payload = ?4,
                                finished_at = ?5, updated_at = ?5
                         WHERE id = ?1 AND status = 1 AND is_deleted = 0",
                        params![
                            &parent_run_id,
                            WorkflowNodeStatus::Succeeded.database_value(),
                            &serialized_output,
                            // Merge onto the Loop row's own payload so the pre-loop checkpoint
                            // recorded before the rounds ran survives the Loop's completion.
                            merge_complete_payload(
                                parent_payload,
                                Some("loop_succeeded".into()),
                                vec![],
                            )?,
                            now,
                        ],
                    )?;
                    transaction.execute(
                        "UPDATE workflow_runs SET payload = ?2, updated_at = ?3 WHERE id = ?1",
                        params![&run_id, serde_json::to_string(&payload)?, now],
                    )?;
                    rewrite_current_nodes(
                        &transaction,
                        &WorkflowRunId::new(run_id),
                        now,
                        |current_nodes| current_nodes.retain(|id| id != &parent_node_id),
                    )?;
                }
                LoopRoundAdvance::Fail { error } => {
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?2, error = ?3,
                                finished_at = ?4, updated_at = ?4
                         WHERE scope_id = ?1 AND status IN (0, 1) AND is_deleted = 0",
                        params![
                            scope_id.as_ref(),
                            WorkflowNodeStatus::Failed.database_value(),
                            error,
                            now,
                        ],
                    )?;
                    settle(&transaction, scope_id, WorkflowScopeStatus::Failed, now)?;
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?2, error = ?3,
                                finished_at = ?4, updated_at = ?4
                         WHERE id = ?1 AND status = 1 AND is_deleted = 0",
                        params![
                            &parent_run_id,
                            WorkflowNodeStatus::Failed.database_value(),
                            error,
                            now,
                        ],
                    )?;
                    transaction.execute(
                        "UPDATE workflow_runs SET run_status = ?2, error = ?3,
                                finished_at = ?4, updated_at = ?4
                         WHERE id = ?1 AND run_status = 1 AND is_deleted = 0",
                        params![
                            &run_id,
                            WorkflowRunStatus::Failed.database_value(),
                            error,
                            now,
                        ],
                    )?;
                }
            }
            transaction.commit()?;
            Ok(AdvanceWorkflowRunResult::Advanced)
        })
        .map_err(engine_repository_error_from_database)
}

/// Moves one active Loop scope to its terminal status within a larger transaction.
fn settle(
    transaction: &Transaction<'_>,
    scope_id: &WorkflowScopeId,
    status: WorkflowScopeStatus,
    now: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE workflow_execution_scopes SET status = ?2, updated_at = ?3
         WHERE id = ?1 AND status = 1",
        params![scope_id.as_ref(), status.database_value(), now],
    )?;
    Ok(())
}

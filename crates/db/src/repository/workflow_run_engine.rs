use ora_application::{
    AdvanceWorkflowRunResult, BindWorkflowNodeSessionResult, CancelWorkflowRunResult,
    ExecutionContext, FailurePropagation, FileChange, IterationRoundContinuation, LoopRoundAdvance,
    LoopRoundExecutionState, LoopRoundToStart, NodeFailure, NodeRunToStart, RepositoryError,
    RestartWorkflowRunResult, ResumeWorkflowRunResult, RoundOutcome, StartWorkflowRunResult,
    UpdateWorkflowRunInputResult, WorkflowRunEngineRepository, WorkflowRunPayload,
};
use ora_domain::{
    SessionId, SessionStatus, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
    WorkflowRunId, WorkflowRunStatus, WorkflowScopeId, WorkflowScopeStatus, WorkflowSnapshotId,
};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};

mod ai_diagnosis;
mod bind;
mod cancel;
mod current_nodes;
mod failure_detail;
mod finish;
mod iteration;
mod loop_round;
mod payload;
mod payload_json;
mod restart;
mod resume;
mod scoped_node;
mod snapshot_switch;

use super::workflow_run::map_run_row;
use super::workspace::{map_workspace_row, workspace_select_sql};
use crate::repository::RepositoryPool;
use current_nodes::{current_nodes_from_state, current_nodes_to_state, rewrite_current_nodes};
use failure_detail::{INTERRUPTED_BY_RESTART, fail_orphaned_run};
use iteration::update_run_execution_state;
use payload::{
    mirror_run_input_into_pool, reset_run_execution_state, seed_system_variables,
    update_task_input_in_payload,
};

/// Persists workflow-run engine state transitions in SQLite.
///
/// The engine repository is separate from the run CRUD repository: it owns node-run writes and
/// the run state machine, and every transition runs in one immediate transaction.
#[derive(Clone, Debug)]
pub struct SqliteWorkflowRunEngineRepository {
    pool: RepositoryPool,
}

impl SqliteWorkflowRunEngineRepository {
    /// Builds an engine repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl WorkflowRunEngineRepository for SqliteWorkflowRunEngineRepository {
    fn find_execution_context(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Option<ExecutionContext>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let run = {
                    let mut statement = connection.prepare(
                        "SELECT wr.id, wr.workspace_id, wr.workflow_id, wr.snapshot_id, wr.name, wr.run_status, wr.state, wr.input, wr.output, wr.error, wr.payload, wr.started_at, wr.finished_at, wr.created_at, wr.updated_at, wr.is_deleted
                         FROM workflow_runs wr WHERE wr.id = ?1 AND wr.is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![run_id.as_ref()])?;
                    match rows.next()?.map(map_run_row).transpose()? {
                        Some(run) => run,
                        None => return Ok(None),
                    }
                };
                let workspace = {
                    let mut statement = connection.prepare(&format!(
                        "{} WHERE w.id = ?1 AND w.is_deleted = 0",
                        workspace_select_sql()
                    ))?;
                    let mut rows = statement.query(params![run.workspace_id.as_ref()])?;
                    match rows.next()? {
                        Some(row) => map_workspace_row(row)?,
                        None => return Ok(None),
                    }
                };
                let graph_json = {
                    let mut statement = connection.prepare(
                        "SELECT graph FROM workflow_snapshots WHERE id = ?1 AND is_deleted = 0",
                    )?;
                    require_row(
                        &mut statement.query(params![run.snapshot_id.as_ref()])?,
                        |row| Ok(row.get::<_, String>(0)?),
                    )?
                };
                Ok(Some(ExecutionContext {
                    root_scope_id: super::workflow_scope::current_root_scope(connection, &run.id)?,
                    run,
                    workspace,
                    graph_json,
                }))
            })
            .map_err(engine_repository_error_from_database)
    }

    fn list_node_runs(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| super::workflow_run::list_node_runs(connection, run_id))
            .map_err(engine_repository_error_from_database)
    }

    fn find_last_failed_attempt(
        &self,
        run_id: &WorkflowRunId,
        node_id: &str,
        iteration: Option<u32>,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                super::workflow_run::find_last_failed_attempt(
                    connection, run_id, node_id, iteration,
                )
            })
            .map_err(engine_repository_error_from_database)
    }

    fn find_active_loop_round(
        &self,
        parent_loop_node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<ora_domain::WorkflowExecutionScope>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                super::workflow_scope::find_active_round(connection, parent_loop_node_run_id)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn list_node_runs_in_scope(
        &self,
        scope_id: &WorkflowScopeId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                super::workflow_run::list_node_runs_in_scope(connection, scope_id)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn bind_node_run_session(
        &self,
        node_run_id: &WorkflowNodeRunId,
        session_id: &SessionId,
        now: i64,
    ) -> Result<BindWorkflowNodeSessionResult, RepositoryError> {
        bind::bind_node_run_session(&self.pool, node_run_id, session_id, now)
    }

    fn find_node_run_by_session_id(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                super::workflow_run::find_node_run_by_session_id(connection, session_id)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn find_node_run_by_id(
        &self,
        node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                super::workflow_run::find_node_run_by_id(connection, node_run_id)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn transition_node_run_status(
        &self,
        node_run_id: &WorkflowNodeRunId,
        from: WorkflowNodeStatus,
        to: WorkflowNodeStatus,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let updated = transaction.execute(
                    "UPDATE workflow_node_runs SET status = ?3, updated_at = ?4
                     WHERE id = ?1 AND status = ?2 AND is_deleted = 0",
                    params![
                        node_run_id.as_ref(),
                        from.database_value(),
                        to.database_value(),
                        now
                    ],
                )?;
                if updated > 0 {
                    transaction.commit()?;
                    return Ok(AdvanceWorkflowRunResult::Advanced);
                }
                // The guard rejected the update: distinguish a missing row from one in another
                // status so a stale flip is a clean no-op rather than a misleading success.
                let exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflow_node_runs WHERE id = ?1 AND is_deleted = 0",
                        params![node_run_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                Ok(if exists {
                    AdvanceWorkflowRunResult::NotRunning
                } else {
                    AdvanceWorkflowRunResult::NotFound
                })
            })
            .map_err(engine_repository_error_from_database)
    }

    fn start_run(
        &self,
        run_id: &WorkflowRunId,
        start_node_run: &NodeRunToStart,
        now: i64,
    ) -> Result<StartWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((status, state, payload, workflow_id)) = transaction
                    .query_row(
                        "SELECT run_status, state, payload, workflow_id FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Ok(StartWorkflowRunResult::NotFound);
                };
                let status = WorkflowRunStatus::from_database_value(status)?;
                let current_nodes = current_nodes_from_state(state.as_deref())?;
                if status != WorkflowRunStatus::Pending || !current_nodes.is_empty() {
                    return Ok(StartWorkflowRunResult::Current);
                }
                insert_node_run(&transaction, run_id, start_node_run, now)?;
                let state = current_nodes_to_state(std::slice::from_ref(&start_node_run.node_id))?;
                transaction.execute(
                    "UPDATE workflow_runs SET run_status = ?2, state = ?3, started_at = ?4, updated_at = ?4
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        run_id.as_ref(),
                        WorkflowRunStatus::Running.database_value(),
                        state,
                        now,
                    ],
                )?;
                // Refresh the system globals whenever a run begins executing. A restart clears every
                // computed pool value first, so without re-seeding here a prompt referencing
                // `sys.timestamp` or `sys.workflow_id` would fail to render on the next run.
                if let Some(serialized_payload) = payload.as_deref() {
                    let mut parsed: WorkflowRunPayload =
                        serde_json::from_str(serialized_payload)?;
                    if seed_system_variables(&mut parsed.variable_pool, &workflow_id, now)? {
                        transaction.execute(
                            "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1 AND is_deleted = 0",
                            params![run_id.as_ref(), serde_json::to_string(&parsed)?],
                        )?;
                    }
                }
                transaction.commit()?;
                Ok(StartWorkflowRunResult::Started)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn start_ready_nodes(
        &self,
        run_id: &WorkflowRunId,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                for node_run in node_runs {
                    insert_node_run(&transaction, run_id, node_run, now)?;
                }
                rewrite_current_nodes(&transaction, run_id, now, |current_nodes| {
                    // Region rows never enter the outer anchor: the composite node stays the
                    // anchor for its whole region (ADR "iteration composite runtime" Stage B).
                    current_nodes.extend(
                        node_runs
                            .iter()
                            .filter(|node_run| node_run.iteration.is_none())
                            .map(|node_run| node_run.node_id.clone()),
                    );
                })?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(engine_repository_error_from_database)
    }

    fn start_loop_round(
        &self,
        run_id: &WorkflowRunId,
        round: &LoopRoundToStart,
        now: i64,
    ) -> Result<(), RepositoryError> {
        loop_round::start(self, run_id, round, now)
    }

    fn start_scope_ready_nodes(
        &self,
        scope_id: &WorkflowScopeId,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<(), RepositoryError> {
        loop_round::start_ready_nodes(self, scope_id, node_runs, now)
    }

    fn advance_loop_round(
        &self,
        scope_id: &WorkflowScopeId,
        advance: &LoopRoundAdvance,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        loop_round::advance(self, scope_id, advance, now)
    }

    fn complete_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        structured_output: Option<serde_json::Value>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        scoped_node::complete(
            self,
            node_run_id,
            output,
            structured_output,
            stop_reason,
            file_changes,
            now,
        )
    }

    fn fail_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
        propagation: FailurePropagation,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        scoped_node::fail(self, node_run_id, failure, propagation, now)
    }

    fn start_iteration_round(
        &self,
        run_id: &WorkflowRunId,
        owner_node_id: &str,
        round: u32,
        item: &serde_json::Value,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.iteration_start_round(run_id, owner_node_id, round, item, node_runs, now)
    }

    fn settle_iteration_round(
        &self,
        run_id: &WorkflowRunId,
        owner_node_id: &str,
        round: u32,
        entry: RoundOutcome,
        continuation: IterationRoundContinuation,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.iteration_settle_round(run_id, owner_node_id, round, entry, continuation, now)
    }

    fn complete_iteration_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        owner_node_id: &str,
        exposed: &[(String, serde_json::Value)],
        output: Option<String>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.iteration_complete_node(node_run_id, owner_node_id, exposed, output, now)
    }

    fn record_node_checkpoint(
        &self,
        node_run_id: &WorkflowNodeRunId,
        snapshot_id: &str,
        checkpoint: Option<&str>,
        checkpoint_error: Option<&str>,
        now: i64,
    ) -> Result<(), RepositoryError> {
        payload_json::record_node_checkpoint(
            &self.pool,
            node_run_id,
            snapshot_id,
            checkpoint,
            checkpoint_error,
            now,
        )
    }

    fn record_node_injected_failure(
        &self,
        node_run_id: &WorkflowNodeRunId,
        text: &str,
    ) -> Result<(), RepositoryError> {
        payload_json::record_node_injected_failure(&self.pool, node_run_id, text)
    }

    fn record_node_ai_diagnosis(
        &self,
        node_run_id: &WorkflowNodeRunId,
        diagnosis_json: &str,
        now: i64,
    ) -> Result<(), RepositoryError> {
        ai_diagnosis::record_node_ai_diagnosis(&self.pool, node_run_id, diagnosis_json, now)
    }

    fn finish_run(
        &self,
        run_id: &WorkflowRunId,
        output: Option<String>,
        now: i64,
    ) -> Result<(), RepositoryError> {
        finish::finish_run(&self.pool, run_id, output, now)
    }

    fn cancel_run(
        &self,
        run_id: &WorkflowRunId,
        now: i64,
    ) -> Result<CancelWorkflowRunResult, RepositoryError> {
        cancel::cancel_run(&self.pool, run_id, now)
    }

    fn restart_run(
        &self,
        run_id: &WorkflowRunId,
        now: i64,
    ) -> Result<RestartWorkflowRunResult, RepositoryError> {
        restart::restart_run(&self.pool, run_id, now)
    }

    fn resume_from_failure(
        &self,
        run_id: &WorkflowRunId,
        node_ids_to_clear: &[String],
        now: i64,
    ) -> Result<ResumeWorkflowRunResult, RepositoryError> {
        resume::resume_from_failure(&self.pool, run_id, node_ids_to_clear, now)
    }

    fn switch_run_snapshot(
        &self,
        run_id: &WorkflowRunId,
        snapshot_id: &WorkflowSnapshotId,
        payload_json: &str,
        now: i64,
    ) -> Result<bool, RepositoryError> {
        snapshot_switch::switch_run_snapshot(&self.pool, run_id, snapshot_id, payload_json, now)
    }

    fn update_run_input(
        &self,
        run_id: &WorkflowRunId,
        input: Option<String>,
        variables: std::collections::BTreeMap<String, serde_json::Value>,
        now: i64,
    ) -> Result<UpdateWorkflowRunInputResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((status, state, payload)) = transaction
                    .query_row(
                        "SELECT run_status, state, payload FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Ok(UpdateWorkflowRunInputResult::NotFound);
                };
                let status = WorkflowRunStatus::from_database_value(status)?;
                let current_nodes = current_nodes_from_state(state.as_deref())?;
                // The kickoff input is frozen only while the run is executing: a `Running` run (or
                // a `Pending` pause with in-flight nodes) is using it, but a not-started `Pending`
                // run and any terminal run may be edited to prepare the next execution.
                let editable = (status == WorkflowRunStatus::Pending && current_nodes.is_empty())
                    || matches!(
                        status,
                        WorkflowRunStatus::Succeeded
                            | WorkflowRunStatus::Failed
                            | WorkflowRunStatus::Cancelled
                    );
                if !editable {
                    return Ok(UpdateWorkflowRunInputResult::NotEditable);
                }
                let payload = update_task_input_in_payload(
                    payload.as_deref(),
                    &variables,
                )?;
                // Keep the reserved `{start_id}.input` selector in sync with the dedicated run
                // instruction column so template references render the text the user just set.
                let payload = mirror_run_input_into_pool(payload.as_deref(), input.as_deref())?;
                transaction.execute(
                    "UPDATE workflow_runs SET input = ?2, payload = ?3, updated_at = ?4
                     WHERE id = ?1 AND is_deleted = 0",
                    params![run_id.as_ref(), input, payload, now],
                )?;
                transaction.commit()?;
                Ok(UpdateWorkflowRunInputResult::Updated)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn list_recoverable_runs(&self) -> Result<Vec<WorkflowRunId>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id FROM workflow_runs WHERE run_status IN (?1, ?2) AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![
                    WorkflowRunStatus::Running.database_value(),
                    WorkflowRunStatus::Failed.database_value(),
                ])?;
                let mut run_ids = Vec::new();
                while let Some(row) = rows.next()? {
                    run_ids.push(WorkflowRunId::new(row.get::<_, String>("id")?));
                }
                Ok(run_ids)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn fail_interrupted_node_runs(
        &self,
        run_id: &WorkflowRunId,
        node_run_ids: &[WorkflowNodeRunId],
        now: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                for node_run_id in node_run_ids {
                    // Only a `Running` row can be an interrupted round row; anything else is a
                    // late sweep over already-terminal state and stays untouched.
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?3, error = ?4, finished_at = ?5, updated_at = ?5
                         WHERE id = ?1 AND run_id = ?2 AND status = ?6 AND is_deleted = 0",
                        params![
                            node_run_id.as_ref(),
                            run_id.as_ref(),
                            WorkflowNodeStatus::Failed.database_value(),
                            INTERRUPTED_BY_RESTART,
                            now,
                            WorkflowNodeStatus::Running.database_value(),
                        ],
                    )?;
                }
                transaction.execute(
                    "UPDATE sessions SET status = ?2, updated_at = ?3
                     WHERE workspace_id = (SELECT workspace_id FROM workflow_runs WHERE id = ?1 AND is_deleted = 0)
                       AND status = ?4 AND is_deleted = 0",
                    params![
                        run_id.as_ref(),
                        SessionStatus::Stopped.database_value(),
                        now,
                        SessionStatus::Running.database_value(),
                    ],
                )?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(engine_repository_error_from_database)
    }

    fn fail_orphaned_node_runs(
        &self,
        run_ids: &[WorkflowRunId],
        now: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                for run_id in run_ids {
                    fail_orphaned_run(&transaction, run_id, now)?;
                }
                transaction.commit()?;
                Ok(())
            })
            .map_err(engine_repository_error_from_database)
    }
}

/// Inserts one node-run row in the `Running` status within the active transaction.
fn insert_node_run(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    node_run: &NodeRunToStart,
    now: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, session_id, status, input, output, error, payload, iteration, started_at, finished_at, created_at, updated_at, is_deleted, scope_id)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, NULL, NULL, NULL, ?7, ?8, NULL, ?8, ?8, 0, ?9)",
        params![
            node_run.id.as_ref(),
            run_id.as_ref(),
            &node_run.node_id,
            &node_run.node_type,
            WorkflowNodeStatus::Running.database_value(),
            node_run.input.as_deref(),
            node_run.iteration,
            now,
            node_run.scope_id.as_ref(),
        ],
    )?;
    Ok(())
}

/// Loads the single row the execution context requires, treating absence as corruption.
fn require_row<T>(
    rows: &mut rusqlite::Rows<'_>,
    map: impl FnOnce(&Row<'_>) -> Result<T, crate::DatabaseError>,
) -> Result<T, crate::DatabaseError> {
    match rows.next()?.map(map).transpose()? {
        Some(value) => Ok(value),
        None => Err(crate::DatabaseError::IncompleteWorkflowRunContext),
    }
}

/// Converts database failures into application-port errors.
fn engine_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_application::WorkflowVariablePool;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    const START_PAYLOAD: &str = r#"{
        "locale":"zh-CN",
        "skillMaterialization":{"bindings":[]},
        "variablePool":{
            "revision":0,
            "catalog":{"start.count":{"valueType":"integer","writer":"start"}},
            "values":{}
        },
        "startNodeId":"start"
    }"#;

    /// Payload whose catalog declares the reserved `start.input` instruction selector.
    const START_PAYLOAD_WITH_INPUT: &str = r#"{
        "locale":"zh-CN",
        "skillMaterialization":{"bindings":[]},
        "variablePool":{
            "revision":0,
            "catalog":{
                "start.input":{"valueType":"string","writer":"start"},
                "start.count":{"valueType":"integer","writer":"start"}
            },
            "values":{}
        },
        "startNodeId":"start"
    }"#;

    /// Updating a pending run writes only explicitly declared Start variables into the pool.
    #[test]
    fn updates_declared_start_variables_without_instruction_aliases() {
        let updated = update_task_input_in_payload(
            Some(START_PAYLOAD),
            &BTreeMap::from([("count".to_string(), json!(3))]),
        )
        .unwrap()
        .unwrap();
        let payload: WorkflowRunPayload = serde_json::from_str(&updated).unwrap();

        assert_eq!(
            payload.variable_pool.values,
            BTreeMap::from([("start.count".to_string(), json!(3))])
        );
    }

    /// JSON null deliberately removes a previously assigned deployment value.
    #[test]
    fn clears_an_optional_start_variable() {
        let seeded = update_task_input_in_payload(
            Some(START_PAYLOAD),
            &BTreeMap::from([("count".to_string(), json!(3))]),
        )
        .unwrap()
        .unwrap();
        let updated = update_task_input_in_payload(
            Some(&seeded),
            &BTreeMap::from([("count".to_string(), Value::Null)]),
        )
        .unwrap()
        .unwrap();
        let payload: WorkflowRunPayload = serde_json::from_str(&updated).unwrap();

        assert_eq!(payload.variable_pool.values.get("start.count"), None);
    }

    /// Deployment cannot assign a string to an integer declaration.
    #[test]
    fn rejects_a_start_value_with_the_wrong_type() {
        assert!(
            update_task_input_in_payload(
                Some(START_PAYLOAD),
                &BTreeMap::from([("count".to_string(), json!("three"))]),
            )
            .is_err()
        );
    }

    /// Updating the run instruction keeps the reserved selector in sync with the input column.
    #[test]
    fn mirrors_a_run_instruction_into_the_reserved_start_input_selector() {
        let updated =
            mirror_run_input_into_pool(Some(START_PAYLOAD_WITH_INPUT), Some("review main"))
                .unwrap()
                .unwrap();
        let payload: WorkflowRunPayload = serde_json::from_str(&updated).unwrap();

        assert_eq!(
            payload.variable_pool.values.get("start.input"),
            Some(&json!("review main"))
        );
        assert_eq!(payload.variable_pool.values.get("start.count"), None);
    }

    /// Clearing the run instruction unsets the selector but keeps it declared for later runs.
    #[test]
    fn clearing_the_run_instruction_unsets_start_input_but_keeps_it_declared() {
        let seeded = mirror_run_input_into_pool(Some(START_PAYLOAD_WITH_INPUT), Some("draft"))
            .unwrap()
            .unwrap();
        let updated = mirror_run_input_into_pool(Some(&seeded), None)
            .unwrap()
            .unwrap();
        let payload: WorkflowRunPayload = serde_json::from_str(&updated).unwrap();

        assert_eq!(payload.variable_pool.values.get("start.input"), None);
        assert!(payload.variable_pool.catalog.contains_key("start.input"));
    }

    /// A payload whose catalog never declared `start.input` leaves the pool untouched.
    #[test]
    fn ignores_a_run_instruction_when_start_input_is_not_declared() {
        let updated = mirror_run_input_into_pool(Some(START_PAYLOAD), Some("review main"))
            .unwrap()
            .unwrap();
        let payload: WorkflowRunPayload = serde_json::from_str(&updated).unwrap();

        assert!(payload.variable_pool.values.is_empty());
    }

    /// Starting a run restores the `sys.*` seeds that a restart clears from the pool.
    #[test]
    fn seeds_system_variables_into_a_declared_catalog() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("sys.workflow_id", "string", "sys");
        pool.declare("sys.timestamp", "number", "sys");

        assert!(seed_system_variables(&mut pool, "workflow-a", 1_700_000_000).unwrap());

        assert_eq!(
            pool.values.get("sys.workflow_id"),
            Some(&json!("workflow-a"))
        );
        assert_eq!(
            pool.values.get("sys.timestamp"),
            Some(&json!(1_700_000_000))
        );
    }

    /// Seeding a pool whose catalog omits the system globals is a no-op.
    #[test]
    fn seeding_system_variables_without_a_catalog_is_a_no_op() {
        let mut pool = WorkflowVariablePool::default();

        assert!(!seed_system_variables(&mut pool, "workflow-a", 1).unwrap());
        assert!(pool.values.is_empty());
    }
}

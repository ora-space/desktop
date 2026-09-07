use ora_application::{
    AdvanceWorkflowRunResult, BindWorkflowNodeSessionResult, CancelWorkflowRunResult,
    ExecutionContext, FileChange, NodeRunToStart, RepositoryError, RestartWorkflowRunResult,
    StartWorkflowRunResult, UpdateWorkflowRunInputResult, WorkflowRunEngineRepository,
    WorkflowRunPayload, WorkflowVariablePool,
};
use ora_domain::{
    SessionId, SessionStatus, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
    WorkflowRunId, WorkflowRunStatus,
};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};

use super::workflow_run::{map_node_run_row, map_run_row};
use super::workspace::{map_workspace_row, workspace_select_sql};
use crate::repository::RepositoryPool;

/// Error written to node runs and runs interrupted by a backend restart.
const INTERRUPTED_BY_RESTART: &str = r#"{"reason":"interrupted_by_restart"}"#;

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

    fn bind_node_run_session(
        &self,
        node_run_id: &WorkflowNodeRunId,
        session_id: &SessionId,
        now: i64,
    ) -> Result<BindWorkflowNodeSessionResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let state = transaction
                    .query_row(
                        "SELECT nr.status, wr.run_status,
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
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .optional()?;
                let Some((node_status, run_status, session_matches_workspace)) = state else {
                    return Ok(BindWorkflowNodeSessionResult::NotFound);
                };
                if session_matches_workspace == 0 {
                    return Ok(BindWorkflowNodeSessionResult::NotFound);
                }
                if WorkflowNodeStatus::from_database_value(node_status)?
                    != WorkflowNodeStatus::Running
                    || WorkflowRunStatus::from_database_value(run_status)?
                        != WorkflowRunStatus::Running
                {
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

    fn find_node_run_by_session_id(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, run_id, node_id, node_type, session_id, status, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted
                     FROM workflow_node_runs
                     WHERE session_id = ?1 AND is_deleted = 0
                     LIMIT 1",
                )?;
                let mut rows = statement.query(params![session_id.as_ref()])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_node_run_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(engine_repository_error_from_database)
    }

    fn find_node_run_by_id(
        &self,
        node_run_id: &WorkflowNodeRunId,
    ) -> Result<Option<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, run_id, node_id, node_type, session_id, status, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted
                     FROM workflow_node_runs
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![node_run_id.as_ref()])?;
                match rows.next()? {
                    Some(row) => Ok(Some(map_node_run_row(row)?)),
                    None => Ok(None),
                }
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
                    current_nodes.extend(node_runs.iter().map(|node_run| node_run.node_id.clone()));
                })?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(engine_repository_error_from_database)
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
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((run_id, node_id, node_type, status, run_payload)) = transaction
                    .query_row(
                        "SELECT nr.run_id, nr.node_id, nr.node_type, nr.status, wr.payload
                         FROM workflow_node_runs nr
                         JOIN workflow_runs wr ON wr.id = nr.run_id
                         WHERE nr.id = ?1 AND nr.is_deleted = 0 AND wr.is_deleted = 0",
                        params![node_run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, Option<String>>(4)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Ok(AdvanceWorkflowRunResult::NotFound);
                };
                // A `Pending` node run is an awaiting interactive node and is completed the same
                // way as a running one; any other status is a late or duplicate callback.
                if !matches!(
                    WorkflowNodeStatus::from_database_value(status)?,
                    WorkflowNodeStatus::Running | WorkflowNodeStatus::Pending
                ) {
                    return Ok(AdvanceWorkflowRunResult::NotRunning);
                }
                let payload = complete_payload(stop_reason, file_changes);
                update_run_execution_state(
                    &transaction,
                    &run_id,
                    &node_id,
                    &node_type,
                    output.as_deref(),
                    structured_output.as_ref(),
                    run_payload.as_deref(),
                )?;
                // A Condition's selected branch is private scheduler state, not node output.
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
                let run_id = WorkflowRunId::new(run_id);
                rewrite_current_nodes(&transaction, &run_id, now, |current_nodes| {
                    current_nodes.retain(|id| id != &node_id);
                })?;
                transaction.commit()?;
                Ok(AdvanceWorkflowRunResult::Advanced)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn fail_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((run_id, node_id, status)) = transaction
                    .query_row(
                        "SELECT run_id, node_id, status FROM workflow_node_runs WHERE id = ?1 AND is_deleted = 0",
                        params![node_run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)?,
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
                transaction.execute(
                    "UPDATE workflow_node_runs SET status = ?2, error = ?3, output = ?4, finished_at = ?5, updated_at = ?5
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        node_run_id.as_ref(),
                        WorkflowNodeStatus::Failed.database_value(),
                        &error,
                        output,
                        now,
                    ],
                )?;
                let run_id = WorkflowRunId::new(run_id);
                rewrite_current_nodes(&transaction, &run_id, now, |current_nodes| {
                    current_nodes.clear();
                    current_nodes.push(node_id.clone());
                })?;
                transaction.execute(
                    "UPDATE workflow_runs SET run_status = ?2, error = ?3, finished_at = ?4, updated_at = ?4
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        run_id.as_ref(),
                        WorkflowRunStatus::Failed.database_value(),
                        error,
                        now,
                    ],
                )?;
                transaction.commit()?;
                Ok(AdvanceWorkflowRunResult::Advanced)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn finish_run(
        &self,
        run_id: &WorkflowRunId,
        output: Option<String>,
        now: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
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

    fn cancel_run(
        &self,
        run_id: &WorkflowRunId,
        now: i64,
    ) -> Result<CancelWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
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
                    params![run_id.as_ref(), WorkflowNodeStatus::Cancelled.database_value(), now],
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
                transaction.commit()?;
                Ok(CancelWorkflowRunResult::Cancelled)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn restart_run(
        &self,
        run_id: &WorkflowRunId,
        now: i64,
    ) -> Result<RestartWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
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
                transaction.commit()?;
                Ok(RestartWorkflowRunResult::Restarted)
            })
            .map_err(engine_repository_error_from_database)
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

    fn fail_orphaned_node_runs(
        &self,
        run_ids: &[WorkflowRunId],
        now: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                for run_id in run_ids {
                    // An awaiting (`Pending`) node is parked on human input, not computing: a
                    // restart must not destroy it. Only a run that has a `Running` (actively
                    // generating) node fails, and it takes every non-terminal node with it.
                    let has_generating: bool = transaction.query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM workflow_node_runs
                            WHERE run_id = ?1 AND status = ?2 AND is_deleted = 0
                         )",
                        params![
                            run_id.as_ref(),
                            WorkflowNodeStatus::Running.database_value()
                        ],
                        |row| row.get(0),
                    )?;
                    if !has_generating {
                        continue;
                    }
                    transaction.execute(
                        "UPDATE workflow_node_runs SET status = ?2, error = ?3, finished_at = ?4, updated_at = ?4
                         WHERE run_id = ?1 AND status IN (0, 1) AND is_deleted = 0",
                        params![
                            run_id.as_ref(),
                            WorkflowNodeStatus::Failed.database_value(),
                            INTERRUPTED_BY_RESTART,
                            now,
                        ],
                    )?;
                    transaction.execute(
                        "UPDATE workflow_runs SET run_status = ?2, error = ?3, finished_at = ?4, updated_at = ?4
                         WHERE id = ?1 AND run_status = ?5 AND is_deleted = 0",
                        params![
                            run_id.as_ref(),
                            WorkflowRunStatus::Failed.database_value(),
                            INTERRUPTED_BY_RESTART,
                            now,
                            WorkflowRunStatus::Running.database_value(),
                        ],
                    )?;
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
                }
                transaction.commit()?;
                Ok(())
            })
            .map_err(engine_repository_error_from_database)
    }
}

/// Reads the `current_nodes` anchor from a run state JSON, treating a null state as empty.
fn current_nodes_from_state(state: Option<&str>) -> Result<Vec<String>, crate::DatabaseError> {
    let Some(state) = state else {
        return Ok(Vec::new());
    };
    let value: serde_json::Value = serde_json::from_str(state)?;
    Ok(value
        .get("current_nodes")
        .and_then(serde_json::Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// Serializes a `current_nodes` anchor into the run state JSON.
fn current_nodes_to_state(current_nodes: &[String]) -> Result<String, crate::DatabaseError> {
    serde_json::to_string(&serde_json::json!({ "current_nodes": current_nodes }))
        .map_err(Into::into)
}

/// Builds the node-run `payload` blob: the ACP stop reason and incremental file changes, when any.
fn complete_payload(stop_reason: Option<String>, file_changes: Vec<FileChange>) -> Option<String> {
    let mut payload = serde_json::Map::new();
    if let Some(reason) = stop_reason {
        payload.insert("stop_reason".to_string(), serde_json::json!(reason));
    }
    if !file_changes.is_empty() {
        payload.insert(
            "file_changes".to_string(),
            serde_json::json!(
                file_changes
                    .iter()
                    .map(|change| {
                        serde_json::json!({
                            "path": change.path,
                            "additions": change.additions,
                            "deletions": change.deletions,
                        })
                    })
                    .collect::<Vec<_>>()
            ),
        );
    }
    if payload.is_empty() {
        return None;
    }
    Some(serde_json::Value::Object(payload).to_string())
}

/// Commits public node values and private routing state with the node status transition.
///
/// Keeping Condition decisions outside the variable pool prevents scheduler implementation details
/// from becoming selectable workflow data while preserving restart-safe branch projection.
fn update_run_execution_state(
    transaction: &Transaction<'_>,
    run_id: &str,
    node_id: &str,
    node_type: &str,
    output: Option<&str>,
    structured_output: Option<&serde_json::Value>,
    serialized_payload: Option<&str>,
) -> Result<(), crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(());
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    let mut changed = if node_type != "condition"
        && let Some(output) = output
    {
        write_pool_variable(
            &mut payload.variable_pool,
            &format!("{node_id}.output"),
            node_id,
            serde_json::Value::String(output.to_string()),
        )?
    } else {
        false
    };
    match node_type {
        "start" => {}
        "condition" => {
            if let Some(output) = output {
                changed |=
                    payload.condition_decisions.get(node_id).map(String::as_str) != Some(output);
                payload
                    .condition_decisions
                    .insert(node_id.to_string(), output.to_string());
            }
        }
        "agent" => {
            if let Some(structured) = structured_output {
                changed |= write_pool_variable(
                    &mut payload.variable_pool,
                    &format!("{node_id}.structured_output"),
                    node_id,
                    structured.clone(),
                )?;
            }
        }
        _ => {}
    }
    if changed {
        transaction.execute(
            "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1 AND is_deleted = 0",
            params![run_id, serde_json::to_string(&payload)?],
        )?;
    }
    Ok(())
}

/// Writes one pool variable through its declared owner, reporting whether the pool changed.
fn write_pool_variable(
    pool: &mut WorkflowVariablePool,
    selector: &str,
    writer: &str,
    value: serde_json::Value,
) -> Result<bool, rusqlite::Error> {
    if !pool.catalog.contains_key(selector) {
        return Ok(false);
    }
    pool.set(selector, writer, value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(true)
}

/// Re-seeds the run's system globals each time it starts executing.
///
/// Restart clears computed pool values but keeps the catalog, so a fresh start must restore the
/// `sys.*` seeds or prompts that reference them fail as unassigned on the second run.
fn seed_system_variables(
    pool: &mut WorkflowVariablePool,
    workflow_id: &str,
    now: i64,
) -> Result<bool, rusqlite::Error> {
    let mut changed = false;
    changed |= write_pool_variable(
        pool,
        "sys.workflow_id",
        "sys",
        serde_json::Value::String(workflow_id.to_string()),
    )?;
    changed |= write_pool_variable(
        pool,
        "sys.timestamp",
        "sys",
        serde_json::Value::Number(now.into()),
    )?;
    Ok(changed)
}

/// Updates explicit Start variables while the run instruction remains in its dedicated column.
fn update_task_input_in_payload(
    serialized_payload: Option<&str>,
    variables: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Option<String>, crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(None);
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    let start_writer = resolve_start_writer(&payload);
    remove_legacy_instruction_aliases(&mut payload, start_writer.as_deref());
    if let Some(start_writer) = start_writer.as_ref() {
        for (name, value) in variables {
            let selector = format!("{start_writer}.{name}");
            let definition = payload
                .variable_pool
                .catalog
                .get(&selector)
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(format!(
                        "undeclared Start variable {name}"
                    ))
                })?;
            if &definition.writer != start_writer {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "Start variable {name} is not editable"
                ))
                .into());
            }
            if value.is_null() {
                payload.variable_pool.values.remove(&selector);
            } else {
                payload
                    .variable_pool
                    .set(&selector, start_writer, value.clone())
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            }
        }
    } else if !variables.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "run payload has no Start node owner".to_string(),
        )
        .into());
    }
    payload.variable_pool.revision = payload.variable_pool.revision.saturating_add(1);
    Ok(Some(serde_json::to_string(&payload)?))
}

/// Mirrors an updated run instruction into the reserved `{start_id}.input` selector so prompt
/// templates render the same text the dedicated run input column holds. Clearing the instruction
/// unsets the selector; the declared variable itself stays available for future runs.
fn mirror_run_input_into_pool(
    serialized_payload: Option<&str>,
    input: Option<&str>,
) -> Result<Option<String>, crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(None);
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    let Some(start_writer) = resolve_start_writer(&payload) else {
        return Ok(Some(serialized_payload.to_string()));
    };
    let selector = format!("{start_writer}.input");
    if !payload.variable_pool.catalog.contains_key(&selector) {
        return Ok(Some(serialized_payload.to_string()));
    }
    let changed = match input {
        Some(text) => payload
            .variable_pool
            .set(
                &selector,
                &start_writer,
                serde_json::Value::String(text.to_string()),
            )
            .map(|()| true)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
        None => payload.variable_pool.values.remove(&selector).is_some(),
    };
    if changed {
        payload.variable_pool.revision = payload.variable_pool.revision.saturating_add(1);
    }
    Ok(Some(serde_json::to_string(&payload)?))
}

/// Clears computed variables and branch decisions while retaining Start deployment values.
fn reset_run_execution_state(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    serialized_payload: Option<&str>,
) -> Result<(), crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(());
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    if payload.variable_pool.catalog.is_empty() && payload.condition_decisions.is_empty() {
        return Ok(());
    }
    let start_writer = resolve_start_writer(&payload);
    remove_legacy_instruction_aliases(&mut payload, start_writer.as_deref());
    let pool = &mut payload.variable_pool;
    // Explicit deployment variables survive restart; values produced during execution do not.
    pool.values.retain(|selector, _| {
        start_writer.as_ref().is_some_and(|writer| {
            pool.catalog
                .get(selector)
                .is_some_and(|definition| &definition.writer == writer)
        })
    });
    pool.revision = pool.revision.saturating_add(1);
    payload.condition_decisions.clear();
    transaction.execute(
        "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1 AND is_deleted = 0",
        params![run_id.as_ref(), serde_json::to_string(&payload)?],
    )?;
    Ok(())
}

/// Resolves the Start variable owner, falling back to the legacy request declaration on upgrade.
fn resolve_start_writer(payload: &WorkflowRunPayload) -> Option<String> {
    payload.start_node_id.clone().or_else(|| {
        payload
            .variable_pool
            .catalog
            .iter()
            .find_map(|(selector, definition)| {
                selector
                    .ends_with(".request")
                    .then(|| definition.writer.clone())
            })
    })
}

/// Removes historical aliases that incorrectly represented the run instruction as variables.
fn remove_legacy_instruction_aliases(payload: &mut WorkflowRunPayload, start_writer: Option<&str>) {
    payload.variable_pool.catalog.remove("sys.task");
    payload.variable_pool.values.remove("sys.task");
    if let Some(start_writer) = start_writer {
        let request_selector = format!("{start_writer}.request");
        payload.variable_pool.catalog.remove(&request_selector);
        payload.variable_pool.values.remove(&request_selector);
    }
}

/// Rewrites the run's `current_nodes` anchor inside the active transaction.
fn rewrite_current_nodes(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    now: i64,
    mutate: impl FnOnce(&mut Vec<String>),
) -> Result<(), crate::DatabaseError> {
    let state = transaction
        .query_row(
            "SELECT state FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
            params![run_id.as_ref()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let mut current_nodes = current_nodes_from_state(state.as_deref())?;
    mutate(&mut current_nodes);
    transaction.execute(
        "UPDATE workflow_runs SET state = ?2, updated_at = ?3 WHERE id = ?1 AND is_deleted = 0",
        params![
            run_id.as_ref(),
            current_nodes_to_state(&current_nodes)?,
            now
        ],
    )?;
    Ok(())
}

/// Inserts one node-run row in the `Running` status within the active transaction.
fn insert_node_run(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    node_run: &NodeRunToStart,
    now: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, session_id, status, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, NULL, NULL, NULL, ?7, NULL, ?7, ?7, 0)",
        params![
            node_run.id.as_ref(),
            run_id.as_ref(),
            &node_run.node_id,
            &node_run.node_type,
            WorkflowNodeStatus::Running.database_value(),
            node_run.input.as_deref(),
            now,
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

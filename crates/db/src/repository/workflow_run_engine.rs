use ora_application::{
    AdvanceWorkflowRunResult, BindWorkflowNodeSessionResult, CancelWorkflowRunResult,
    ExecutionContext, FileChange, NodeRunToStart, RepositoryError, RestartWorkflowRunResult,
    StartWorkflowRunResult, UpdateWorkflowRunInputResult, WorkflowRunEngineRepository,
};
use ora_domain::{
    SessionId, SessionStatus, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus,
    WorkflowRunId, WorkflowRunStatus,
};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};

use super::task::map_task_row;
use super::workflow_run::{map_node_run_row, map_run_row};
use super::worktree::map_worktree_row;
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
                        "SELECT id, workflow_id, snapshot_id, run_status, state, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted
                         FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![run_id.as_ref()])?;
                    match rows.next()?.map(map_run_row).transpose()? {
                        Some(run) => run,
                        None => return Ok(None),
                    }
                };
                // A run created through create_run always carries its task, worktree, and frozen
                // snapshot; any of them missing is corruption, not a legitimate absence.
                let task = {
                    let mut statement = connection.prepare(
                        "SELECT id, project_id, title, type, workflow_run_id, worktree_id, created_at, updated_at, is_deleted
                         FROM tasks WHERE workflow_run_id = ?1 AND is_deleted = 0",
                    )?;
                    require_row(&mut statement.query(params![run_id.as_ref()])?, map_task_row)?
                };
                let worktree = {
                    let mut statement = connection.prepare(
                        "SELECT id, task_id, branch_name, checkout_root, base_commit_id, is_active, created_at, updated_at, is_deleted
                         FROM worktrees WHERE task_id = ?1 AND is_deleted = 0",
                    )?;
                    require_row(&mut statement.query(params![task.id.as_ref()])?, map_worktree_row)?
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
                    task,
                    worktree,
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
                        "SELECT nr.status, wr.run_status
                         FROM workflow_node_runs nr
                         JOIN workflow_runs wr ON wr.id = nr.run_id
                         WHERE nr.id = ?1 AND nr.is_deleted = 0 AND wr.is_deleted = 0",
                        params![node_run_id.as_ref()],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?;
                let Some((node_status, run_status)) = state else {
                    return Ok(BindWorkflowNodeSessionResult::NotFound);
                };
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
                let Some((status, state)) = transaction
                    .query_row(
                        "SELECT run_status, state FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
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
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
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
                // A `Pending` node run is an awaiting interactive node and is completed the same
                // way as a running one; any other status is a late or duplicate callback.
                if !matches!(
                    WorkflowNodeStatus::from_database_value(status)?,
                    WorkflowNodeStatus::Running | WorkflowNodeStatus::Pending
                ) {
                    return Ok(AdvanceWorkflowRunResult::NotRunning);
                }
                let payload = complete_payload(stop_reason, file_changes);
                transaction.execute(
                    "UPDATE workflow_node_runs SET status = ?2, output = ?3, payload = ?4, finished_at = ?5, updated_at = ?5
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        node_run_id.as_ref(),
                        WorkflowNodeStatus::Succeeded.database_value(),
                        output,
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
                let status = transaction
                    .query_row(
                        "SELECT run_status FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                let Some(status) = status else {
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
                transaction.commit()?;
                Ok(RestartWorkflowRunResult::Restarted)
            })
            .map_err(engine_repository_error_from_database)
    }

    fn update_run_input(
        &self,
        run_id: &WorkflowRunId,
        input: Option<String>,
        now: i64,
    ) -> Result<UpdateWorkflowRunInputResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((status, state)) = transaction
                    .query_row(
                        "SELECT run_status, state FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
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
                transaction.execute(
                    "UPDATE workflow_runs SET input = ?2, updated_at = ?3
                     WHERE id = ?1 AND is_deleted = 0",
                    params![run_id.as_ref(), input, now],
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
                         WHERE task_id = (SELECT id FROM tasks WHERE workflow_run_id = ?1 AND is_deleted = 0)
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

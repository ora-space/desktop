use ora_application::{
    DeleteWorkflowRunResult, RepositoryError, WorkflowRunCreateOutcome, WorkflowRunRepository,
};
use ora_domain::{
    AuditFields, ProjectId, SessionId, SessionStatus, WorkflowId, WorkflowNodeRun,
    WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail, WorkflowRunId,
    WorkflowRunStatus, WorkflowSnapshotId, WorkspaceId,
};
use rusqlite::{OptionalExtension, Row, ToSql, Transaction, TransactionBehavior, params};

use crate::repository::RepositoryPool;

/// Persists workflow runs directly under workspaces and keeps node history under each run.
#[derive(Clone, Debug)]
pub struct SqliteWorkflowRunRepository {
    pool: RepositoryPool,
}

impl SqliteWorkflowRunRepository {
    /// Builds a run repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl WorkflowRunRepository for SqliteWorkflowRunRepository {
    /// Inserts a run after validating its workspace admission in the same transaction.
    fn create_run(&self, run: WorkflowRun) -> Result<WorkflowRunCreateOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let workspace_visible = transaction
                    .query_row(
                        "SELECT 1 FROM workspaces w
                         JOIN projects p ON p.id = w.project_id AND p.is_deleted = 0
                         JOIN workspace_provisioning wp ON wp.workspace_id = w.id
                         WHERE w.id = ?1 AND w.is_deleted = 0 AND w.lifecycle = 'active'
                           AND wp.state = 'ready'",
                        params![run.workspace_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !workspace_visible {
                    return Ok(WorkflowRunCreateOutcome::WorkspaceNotVisible);
                }
                transaction.execute(
                    "INSERT INTO workflow_runs (id, workspace_id, workflow_id, snapshot_id, name, run_status, state, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        run.id.as_ref(),
                        run.workspace_id.as_ref(),
                        run.workflow_id.as_ref(),
                        run.snapshot_id.as_ref(),
                        &run.name,
                        run.status.database_value(),
                        run.state.as_deref(),
                        run.input.as_deref(),
                        run.output.as_deref(),
                        run.error.as_deref(),
                        run.payload.as_deref(),
                        run.started_at,
                        run.finished_at,
                        run.audit_fields.created_at,
                        run.audit_fields.updated_at,
                        i64::from(run.audit_fields.is_deleted),
                    ],
                )?;
                transaction.commit()?;
                Ok(WorkflowRunCreateOutcome::Created(Box::new(run)))
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Loads one visible workflow run without consulting any task row.
    fn find_run(&self, run_id: &WorkflowRunId) -> Result<Option<WorkflowRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(run_select_sql())?;
                let mut rows = statement.query(params![run_id.as_ref()])?;
                rows.next()?.map(map_run_row).transpose()
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Loads a run, its direct workspace identity, its project, and node history.
    fn get_run_detail(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Option<WorkflowRunDetail>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let Some(run) = load_run(connection, run_id)? else {
                    return Ok(None);
                };
                let project_id = connection.query_row(
                    "SELECT project_id FROM workspaces WHERE id = ?1",
                    params![run.workspace_id.as_ref()],
                    |row| row.get::<_, String>(0),
                )?;
                Ok(Some(WorkflowRunDetail {
                    name: run.name.clone(),
                    workspace_id: run.workspace_id.clone(),
                    project_id: ProjectId::new(project_id),
                    nodes: list_node_runs(connection, run_id)?,
                    run,
                }))
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Lists visible runs by their workspace's project without traversing Task.
    fn list_runs_by_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ora_domain::WorkflowRunSummary>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                list_summary_rows(
                    connection,
                    "wr.workspace_id IN (SELECT id FROM workspaces WHERE project_id = ?1 AND is_deleted = 0)",
                    &[&project_id.as_ref()],
                )
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Lists visible runs for one workflow while retaining their direct workspace ids.
    fn list_runs_by_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<ora_domain::WorkflowRunSummary>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                list_summary_rows(connection, "wr.workflow_id = ?1", &[&workflow_id.as_ref()])
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Lists the node-run records of one visible run in stable ascending order.
    fn list_node_runs(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| list_node_runs(connection, run_id))
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Renames a visible run in place while keeping its workspace-owned execution identity.
    fn rename_run(
        &self,
        run_id: &WorkflowRunId,
        name: String,
        updated_at: i64,
    ) -> Result<Option<WorkflowRun>, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let changed = transaction.execute(
                    "UPDATE workflow_runs SET name = ?2, updated_at = ?3
                     WHERE id = ?1 AND is_deleted = 0",
                    params![run_id.as_ref(), name, updated_at],
                )?;
                transaction.commit()?;
                if changed == 0 {
                    Ok(None)
                } else {
                    load_run(connection, run_id)
                }
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    /// Soft-deletes the run, node rows, and sessions owned by its node bindings after refusing active execution.
    fn soft_delete_run(
        &self,
        run_id: &WorkflowRunId,
        deleted_at: i64,
    ) -> Result<DeleteWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some(_workspace_id) = transaction
                    .query_row(
                        "SELECT workspace_id FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                else {
                    return Ok(DeleteWorkflowRunResult::NotFound);
                };
                // `Pending` is both "created, not started" and a HITL pause. Only the
                // latter is active: it has non-terminal node rows. Treating every
                // Pending run as live blocked deleting a run the user created but
                // never executed.
                let active = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM workflow_runs
                        WHERE id = ?1 AND run_status = ?2 AND is_deleted = 0
                    ) OR EXISTS(
                        SELECT 1 FROM workflow_node_runs
                        WHERE run_id = ?1 AND status IN (?3, ?4) AND is_deleted = 0
                    ) OR EXISTS(
                        SELECT 1
                        FROM sessions s
                        JOIN workflow_node_runs n ON n.session_id = s.id
                        WHERE n.run_id = ?1
                          AND n.is_deleted = 0
                          AND s.status = ?5
                          AND s.is_deleted = 0
                    )",
                    params![
                        run_id.as_ref(),
                        WorkflowRunStatus::Running.database_value(),
                        WorkflowNodeStatus::Pending.database_value(),
                        WorkflowNodeStatus::Running.database_value(),
                        SessionStatus::Running.database_value(),
                    ],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                if active {
                    return Ok(DeleteWorkflowRunResult::ActiveRun);
                }
                transaction.execute(
                    "UPDATE workflow_node_runs SET updated_at = ?2, is_deleted = 1
                     WHERE run_id = ?1 AND is_deleted = 0",
                    params![run_id.as_ref(), deleted_at],
                )?;
                transaction.execute(
                    "UPDATE workflow_runs SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![run_id.as_ref(), deleted_at],
                )?;
                // Node-bound sessions are the only sessions this run owns. Keep
                // a session alive when another non-deleted node run still
                // references it, because Workspace sharing makes unrelated
                // sessions independent of this run.
                transaction.execute(
                    "UPDATE sessions SET updated_at = ?2, is_deleted = 1
                     WHERE id IN (
                         SELECT n.session_id
                         FROM workflow_node_runs n
                         WHERE n.run_id = ?1 AND n.session_id IS NOT NULL
                     )
                       AND is_deleted = 0
                       AND NOT EXISTS (
                           SELECT 1 FROM workflow_node_runs other
                           WHERE other.session_id = sessions.id
                             AND other.run_id <> ?1
                             AND other.is_deleted = 0
                       )",
                    params![run_id.as_ref(), deleted_at],
                )?;
                transaction.commit()?;
                Ok(DeleteWorkflowRunResult::Deleted)
            })
            .map_err(workflow_run_repository_error_from_database)
    }
}

/// Returns the run columns shared by the detail and single-run readers.
fn run_select_sql() -> &'static str {
    "SELECT id, workspace_id, workflow_id, snapshot_id, name, run_status, state, input, output,
            error, payload, started_at, finished_at, created_at, updated_at, is_deleted
     FROM workflow_runs
     WHERE id = ?1 AND is_deleted = 0"
}

/// Loads one run from a connection without applying transport mapping.
fn load_run(
    connection: &rusqlite::Connection,
    run_id: &WorkflowRunId,
) -> Result<Option<WorkflowRun>, crate::DatabaseError> {
    let mut statement = connection.prepare(run_select_sql())?;
    let mut rows = statement.query(params![run_id.as_ref()])?;
    rows.next()?.map(map_run_row).transpose()
}

/// Reads summary rows using a caller-supplied workspace or workflow predicate.
fn list_summary_rows(
    connection: &rusqlite::Connection,
    predicate: &str,
    predicate_params: &[&dyn ToSql],
) -> Result<Vec<ora_domain::WorkflowRunSummary>, crate::DatabaseError> {
    let sql = format!(
        "SELECT wr.id, wr.workspace_id, w.project_id, wr.name, wr.workflow_id, wr.run_status,
                wr.started_at, wr.finished_at, wr.created_at,
                EXISTS(SELECT 1 FROM workflow_node_runs n
                       WHERE n.run_id = wr.id AND n.status = 0 AND n.is_deleted = 0) AS has_awaiting_node
         FROM workflow_runs wr
         JOIN workspaces w ON w.id = wr.workspace_id AND w.is_deleted = 0
         WHERE {predicate} AND wr.is_deleted = 0
         ORDER BY wr.created_at ASC, wr.id ASC"
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(predicate_params)?;
    let mut summaries = Vec::new();
    while let Some(row) = rows.next()? {
        summaries.push(ora_domain::WorkflowRunSummary {
            id: WorkflowRunId::new(row.get::<_, String>("id")?),
            name: row.get::<_, String>("name")?,
            workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
            project_id: ProjectId::new(row.get::<_, String>("project_id")?),
            workflow_id: WorkflowId::new(row.get::<_, String>("workflow_id")?),
            status: WorkflowRunStatus::from_database_value(row.get("run_status")?)?,
            has_awaiting_node: row.get("has_awaiting_node")?,
            started_at: row.get("started_at")?,
            finished_at: row.get("finished_at")?,
            created_at: row.get("created_at")?,
        });
    }
    Ok(summaries)
}

/// Reconstructs a domain run from the selected direct-workspace columns.
pub(super) fn map_run_row(row: &Row<'_>) -> Result<WorkflowRun, crate::DatabaseError> {
    Ok(WorkflowRun::new(
        WorkflowRunId::new(row.get::<_, String>("id")?),
        WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        WorkflowId::new(row.get::<_, String>("workflow_id")?),
        WorkflowSnapshotId::new(row.get::<_, String>("snapshot_id")?),
        row.get::<_, String>("name")?,
        WorkflowRunStatus::from_database_value(row.get("run_status")?)?,
        row.get("state")?,
        row.get("input")?,
        row.get("output")?,
        row.get("error")?,
        row.get("payload")?,
        row.get("started_at")?,
        row.get("finished_at")?,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    ))
}

/// Reconstructs a node-run snapshot from one selected row.
pub(super) fn map_node_run_row(row: &Row<'_>) -> Result<WorkflowNodeRun, crate::DatabaseError> {
    Ok(WorkflowNodeRun::new(
        WorkflowNodeRunId::new(row.get::<_, String>("id")?),
        WorkflowRunId::new(row.get::<_, String>("run_id")?),
        row.get::<_, String>("node_id")?,
        row.get::<_, String>("node_type")?,
        row.get::<_, Option<String>>("session_id")?
            .map(SessionId::new),
        WorkflowNodeStatus::from_database_value(row.get("status")?)?,
        row.get("input")?,
        row.get("output")?,
        row.get("error")?,
        row.get("payload")?,
        row.get("started_at")?,
        row.get("finished_at")?,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    ))
}

/// Lists node-run rows of one run in stable ascending order.
pub(super) fn list_node_runs(
    connection: &rusqlite::Connection,
    run_id: &WorkflowRunId,
) -> Result<Vec<WorkflowNodeRun>, crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, run_id, node_id, node_type, session_id, status, input, output, error, payload,
                started_at, finished_at, created_at, updated_at, is_deleted
         FROM workflow_node_runs
         WHERE run_id = ?1 AND is_deleted = 0
         ORDER BY created_at, id",
    )?;
    let mut rows = statement.query(params![run_id.as_ref()])?;
    let mut node_runs = Vec::new();
    while let Some(row) = rows.next()? {
        node_runs.push(map_node_run_row(row)?);
    }
    Ok(node_runs)
}

/// Converts database failures into application-port errors.
fn workflow_run_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

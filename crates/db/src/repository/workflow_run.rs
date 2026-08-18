use ora_application::{
    DeleteWorkflowRunResult, RepositoryError, WorkflowRunCreateOutcome, WorkflowRunRepository,
};
use ora_domain::{
    AuditFields, ProjectId, SessionId, SessionStatus, Task, TaskId, WorkflowId, WorkflowNodeRun,
    WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail, WorkflowRunId,
    WorkflowRunStatus, WorkflowRunSummary, WorkflowSnapshotId, Worktree, WorktreeBaseline,
    WorktreeProvisioningLeaseId,
};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists workflow runs and their node-run history in SQLite.
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
    fn create_run(
        &self,
        run: WorkflowRun,
        task: Task,
        worktree: Worktree,
        lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<WorkflowRunCreateOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                // Same atomic-finish contract as ordinary task creation: a run
                // must not become visible under a project a cascade already
                // removed, and its provisioning lease dies with this commit.
                let project_visible = transaction
                    .query_row(
                        "SELECT 1 FROM projects WHERE id = ?1 AND is_deleted = 0",
                        params![task.project_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !project_visible {
                    return Ok(WorkflowRunCreateOutcome::ProjectNotVisible);
                }
                // The run row must precede the task row: `tasks.workflow_run_id` is an immediate
                // foreign key on `workflow_runs`, so the parent row has to exist first.
                transaction.execute(
                    "INSERT INTO workflow_runs (id, workflow_id, snapshot_id, run_status, state, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        run.id.as_ref(),
                        run.workflow_id.as_ref(),
                        run.snapshot_id.as_ref(),
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
                        bool_to_sqlite(run.audit_fields.is_deleted),
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO tasks (id, project_id, title, type, workflow_run_id, worktree_id, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        task.id.as_ref(),
                        task.project_id.as_ref(),
                        &task.title,
                        task.task_type.database_value(),
                        task.workflow_run_id.as_ref().map(AsRef::as_ref),
                        task.worktree_id.as_ref().map(AsRef::as_ref),
                        task.audit_fields.created_at,
                        task.audit_fields.updated_at,
                        bool_to_sqlite(task.audit_fields.is_deleted),
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO worktrees (id, task_id, branch_name, checkout_root, base_commit_id, is_active, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        worktree.id.as_ref(),
                        worktree.task_id.as_ref(),
                        worktree.branch_name.as_deref(),
                        worktree.checkout_root.as_deref(),
                        baseline_value(&worktree.baseline),
                        worktree.activity.database_value(),
                        worktree.audit_fields.created_at,
                        worktree.audit_fields.updated_at,
                        bool_to_sqlite(worktree.audit_fields.is_deleted),
                    ],
                )?;
                transaction.execute(
                    "DELETE FROM worktree_provisioning_leases WHERE id = ?1",
                    params![lease_id.as_ref()],
                )?;
                transaction.commit()?;
                Ok(WorkflowRunCreateOutcome::Created(Box::new(run)))
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    fn find_run(&self, run_id: &WorkflowRunId) -> Result<Option<WorkflowRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workflow_id, snapshot_id, run_status, state, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted
                     FROM workflow_runs
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![run_id.as_ref()])?;
                rows.next()?.map(map_run_row).transpose()
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    fn get_run_detail(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Option<WorkflowRunDetail>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let run = {
                    let mut statement = connection.prepare(
                        "SELECT id, workflow_id, snapshot_id, run_status, state, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted
                         FROM workflow_runs
                         WHERE id = ?1 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![run_id.as_ref()])?;
                    match rows.next()?.map(map_run_row).transpose()? {
                        Some(run) => run,
                        None => return Ok(None),
                    }
                };
                // The display name, task id, and owning project live on the run-task; a run
                // created through create_run always has one, so an absent row degrades to empty
                // values rather than corruption.
                let (task_id, name, project_id) = connection
                    .query_row(
                        "SELECT id, title, project_id FROM tasks WHERE workflow_run_id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?
                    .unwrap_or_default();
                let nodes = list_node_runs(connection, run_id)?;
                Ok(Some(WorkflowRunDetail {
                    run,
                    name,
                    project_id: ProjectId::new(project_id),
                    task_id: TaskId::new(task_id),
                    nodes,
                }))
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    fn list_runs_by_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT wr.id, t.title AS name, t.project_id, wr.workflow_id, wr.run_status, wr.started_at, wr.finished_at, wr.created_at
                     FROM workflow_runs wr
                     JOIN tasks t ON t.workflow_run_id = wr.id AND t.is_deleted = 0
                     WHERE t.project_id = ?1 AND wr.is_deleted = 0
                     ORDER BY wr.created_at ASC, wr.id ASC",
                )?;
                let mut rows = statement.query(params![project_id.as_ref()])?;
                let mut summaries = Vec::new();
                while let Some(row) = rows.next()? {
                    summaries.push(WorkflowRunSummary {
                        id: WorkflowRunId::new(row.get::<_, String>("id")?),
                        name: row.get::<_, String>("name")?,
                        project_id: ProjectId::new(row.get::<_, String>("project_id")?),
                        workflow_id: WorkflowId::new(row.get::<_, String>("workflow_id")?),
                        status: WorkflowRunStatus::from_database_value(row.get("run_status")?)?,
                        started_at: row.get("started_at")?,
                        finished_at: row.get("finished_at")?,
                        created_at: row.get("created_at")?,
                    });
                }
                Ok(summaries)
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    fn list_runs_by_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT wr.id, t.title AS name, t.project_id, wr.workflow_id, wr.run_status, wr.started_at, wr.finished_at, wr.created_at
                     FROM workflow_runs wr
                     JOIN tasks t ON t.workflow_run_id = wr.id AND t.is_deleted = 0
                     WHERE wr.workflow_id = ?1 AND wr.is_deleted = 0
                     ORDER BY wr.created_at ASC, wr.id ASC",
                )?;
                let mut rows = statement.query(params![workflow_id.as_ref()])?;
                let mut summaries = Vec::new();
                while let Some(row) = rows.next()? {
                    summaries.push(WorkflowRunSummary {
                        id: WorkflowRunId::new(row.get::<_, String>("id")?),
                        name: row.get::<_, String>("name")?,
                        project_id: ProjectId::new(row.get::<_, String>("project_id")?),
                        workflow_id: WorkflowId::new(row.get::<_, String>("workflow_id")?),
                        status: WorkflowRunStatus::from_database_value(row.get("run_status")?)?,
                        started_at: row.get("started_at")?,
                        finished_at: row.get("finished_at")?,
                        created_at: row.get("created_at")?,
                    });
                }
                Ok(summaries)
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    fn list_node_runs(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        self.pool
            .with_connection(|connection| list_node_runs(connection, run_id))
            .map_err(workflow_run_repository_error_from_database)
    }

    fn find_run_task_id(&self, run_id: &WorkflowRunId) -> Result<Option<TaskId>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let task_id = connection
                    .query_row(
                        "SELECT id FROM tasks WHERE workflow_run_id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                Ok(task_id.map(TaskId::new))
            })
            .map_err(workflow_run_repository_error_from_database)
    }

    fn soft_delete_run(
        &self,
        run_id: &WorkflowRunId,
        deleted_at: i64,
    ) -> Result<DeleteWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let run_exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !run_exists {
                    return Ok(DeleteWorkflowRunResult::NotFound);
                }
                // Sessions and worktrees are owned by the run-task, so resolve it for the cascade.
                let task_id = transaction
                    .query_row(
                        "SELECT id FROM tasks WHERE workflow_run_id = ?1 AND is_deleted = 0",
                        params![run_id.as_ref()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;

                let running_run = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM workflow_runs
                        WHERE id = ?1 AND run_status = ?2 AND is_deleted = 0
                    )",
                    params![run_id.as_ref(), WorkflowRunStatus::Running.database_value()],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                let pending_node = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM workflow_node_runs
                        WHERE run_id = ?1 AND status IN (0, 1) AND is_deleted = 0
                    )",
                    params![run_id.as_ref()],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                let running_session = match &task_id {
                    Some(task_id) => {
                        transaction.query_row(
                            "SELECT EXISTS(
                            SELECT 1 FROM sessions
                            WHERE task_id = ?1 AND status = ?2 AND is_deleted = 0
                        )",
                            params![task_id, SessionStatus::Running.database_value()],
                            |row| row.get::<_, i64>(0),
                        )? != 0
                    }
                    None => false,
                };
                if running_run || pending_node || running_session {
                    return Ok(DeleteWorkflowRunResult::ActiveRun);
                }

                // Register the run-task's Git cleanup job in the same transaction
                // as the cascade: physical removal is asynchronous but its intent
                // must not be losable once this delete commits.
                if let Some(task_id) = &task_id {
                    let cleanup_jobs = crate::repository::cascade::collect_task_cleanup_jobs(
                        &transaction,
                        "t.id = ?1",
                        task_id,
                        deleted_at,
                    )?;
                    crate::repository::git_cleanup_job::insert_jobs(&transaction, &cleanup_jobs)?;
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
                if let Some(task_id) = &task_id {
                    transaction.execute(
                        "UPDATE sessions SET updated_at = ?2, is_deleted = 1
                         WHERE task_id = ?1 AND is_deleted = 0",
                        params![task_id, deleted_at],
                    )?;
                    transaction.execute(
                        "UPDATE worktrees SET updated_at = ?2, is_deleted = 1
                         WHERE task_id = ?1 AND is_deleted = 0",
                        params![task_id, deleted_at],
                    )?;
                    transaction.execute(
                        "UPDATE tasks SET updated_at = ?2, is_deleted = 1
                         WHERE id = ?1 AND is_deleted = 0",
                        params![task_id, deleted_at],
                    )?;
                }
                transaction.commit()?;
                Ok(DeleteWorkflowRunResult::Deleted)
            })
            .map_err(workflow_run_repository_error_from_database)
    }
}

/// Reconstructs a domain run from the selected run columns.
pub(super) fn map_run_row(row: &Row<'_>) -> Result<WorkflowRun, crate::DatabaseError> {
    let status = WorkflowRunStatus::from_database_value(row.get("run_status")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(WorkflowRun::new(
        WorkflowRunId::new(row.get::<_, String>("id")?),
        WorkflowId::new(row.get::<_, String>("workflow_id")?),
        WorkflowSnapshotId::new(row.get::<_, String>("snapshot_id")?),
        status,
        row.get("state")?,
        row.get("input")?,
        row.get("output")?,
        row.get("error")?,
        row.get("payload")?,
        row.get("started_at")?,
        row.get("finished_at")?,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Reconstructs a domain node run from the selected node-run columns.
pub(super) fn map_node_run_row(row: &Row<'_>) -> Result<WorkflowNodeRun, crate::DatabaseError> {
    let status = WorkflowNodeStatus::from_database_value(row.get("status")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(WorkflowNodeRun::new(
        WorkflowNodeRunId::new(row.get::<_, String>("id")?),
        WorkflowRunId::new(row.get::<_, String>("run_id")?),
        row.get::<_, String>("node_id")?,
        row.get::<_, String>("node_type")?,
        row.get::<_, Option<String>>("session_id")?
            .map(SessionId::new),
        status,
        row.get("input")?,
        row.get("output")?,
        row.get("error")?,
        row.get("payload")?,
        row.get("started_at")?,
        row.get("finished_at")?,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Lists the node-run rows of one run in stable ascending order.
pub(super) fn list_node_runs(
    connection: &rusqlite::Connection,
    run_id: &WorkflowRunId,
) -> Result<Vec<WorkflowNodeRun>, crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, run_id, node_id, node_type, session_id, status, input, output, error, payload, started_at, finished_at, created_at, updated_at, is_deleted
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

/// Maps the explicit domain baseline state into the nullable migration representation.
fn baseline_value(baseline: &WorktreeBaseline) -> Option<&str> {
    baseline.commit_id()
}

/// Converts database failures into application-port errors.
fn workflow_run_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

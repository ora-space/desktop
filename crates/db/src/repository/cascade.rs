use ora_domain::{GitCleanupJob, GitCleanupJobId, ProjectId, SessionStatus, TaskId, WorkspaceId};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::repository::RepositoryPool;
use crate::repository::git_cleanup_job::insert_jobs;

/// Reports the atomic outcome of an Ora-owned workspace deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeDeleteOutcome {
    Deleted,
    NotFound,
    ActiveSession,
}

/// Performs workspace and project soft deletes in one SQLite transaction without invoking Git.
#[derive(Clone, Debug)]
pub struct SqliteCascadeRepository {
    pool: RepositoryPool,
}

impl SqliteCascadeRepository {
    /// Builds a cascade repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Deletes the workspace represented by one worktree-task label and its descendants.
    pub fn delete_task(
        &self,
        task_id: &TaskId,
        deleted_at: i64,
    ) -> Result<CascadeDeleteOutcome, crate::DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            let workspace_id = transaction
                .query_row(
                    "SELECT workspace_id FROM tasks WHERE id = ?1 AND is_deleted = 0",
                    params![task_id.as_ref()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(workspace_id) = workspace_id else {
                return Ok(CascadeDeleteOutcome::NotFound);
            };
            if workspace_has_running_descendants(&transaction, &workspace_id)? {
                return Ok(CascadeDeleteOutcome::ActiveSession);
            }
            let cleanup_jobs = collect_workspace_cleanup_jobs(
                &transaction,
                "w.id = ?1",
                &workspace_id,
                deleted_at,
            )?;
            insert_jobs(&transaction, &cleanup_jobs)?;
            soft_delete_workspace_descendants(&transaction, &workspace_id, deleted_at)?;
            transaction.execute(
                "UPDATE tasks SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![task_id.as_ref(), deleted_at],
            )?;
            transaction.commit()?;
            Ok(CascadeDeleteOutcome::Deleted)
        })
    }

    /// Deletes all visible workspaces belonging to a project after checking live descendants.
    pub fn delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<CascadeDeleteOutcome, crate::DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM projects WHERE id = ?1 AND is_deleted = 0",
                    params![project_id.as_ref()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(CascadeDeleteOutcome::NotFound);
            }
            let running = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sessions s
                    JOIN workspaces w ON w.id = s.workspace_id
                    WHERE w.project_id = ?1 AND w.is_deleted = 0
                      AND s.status = ?2 AND s.is_deleted = 0
                ) OR EXISTS(
                    SELECT 1 FROM workflow_runs wr
                    JOIN workspaces w ON w.id = wr.workspace_id
                    WHERE w.project_id = ?1 AND w.is_deleted = 0
                      AND wr.run_status IN (0, 1) AND wr.is_deleted = 0
                )",
                params![project_id.as_ref(), SessionStatus::Running.database_value()],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if running {
                return Ok(CascadeDeleteOutcome::ActiveSession);
            }
            let cleanup_jobs = collect_workspace_cleanup_jobs(
                &transaction,
                "w.project_id = ?1",
                project_id.as_ref(),
                deleted_at,
            )?;
            insert_jobs(&transaction, &cleanup_jobs)?;
            let workspace_ids = visible_workspace_ids(&transaction, project_id)?;
            for workspace_id in workspace_ids {
                soft_delete_workspace_descendants(
                    &transaction,
                    workspace_id.as_ref(),
                    deleted_at,
                )?;
            }
            transaction.execute(
                "UPDATE projects SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![project_id.as_ref(), deleted_at],
            )?;
            transaction.commit()?;
            Ok(CascadeDeleteOutcome::Deleted)
        })
    }
}

/// Checks all runtime descendants before a workspace is retired.
fn workspace_has_running_descendants(
    transaction: &Transaction<'_>,
    workspace_id: &str,
) -> Result<bool, rusqlite::Error> {
    transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sessions WHERE workspace_id = ?1 AND status = ?2 AND is_deleted = 0
            ) OR EXISTS(
                SELECT 1 FROM workflow_runs WHERE workspace_id = ?1 AND run_status IN (0, 1) AND is_deleted = 0
            )",
            params![workspace_id, SessionStatus::Running.database_value()],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
}

/// Loads visible workspace ids for the project before their rows are soft-deleted.
fn visible_workspace_ids(
    transaction: &Transaction<'_>,
    project_id: &ProjectId,
) -> Result<Vec<WorkspaceId>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT id FROM workspaces WHERE project_id = ?1 AND is_deleted = 0 ORDER BY created_at, id",
    )?;
    let rows = statement.query_map(params![project_id.as_ref()], |row| {
        Ok(WorkspaceId::new(row.get::<_, String>(0)?))
    })?;
    rows.collect()
}

/// Soft-deletes all database descendants while leaving physical Git cleanup to the durable queue.
fn soft_delete_workspace_descendants(
    transaction: &Transaction<'_>,
    workspace_id: &str,
    deleted_at: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE workflow_node_runs SET updated_at = ?2, is_deleted = 1
         WHERE run_id IN (SELECT id FROM workflow_runs WHERE workspace_id = ?1 AND is_deleted = 0)
           AND is_deleted = 0",
        params![workspace_id, deleted_at],
    )?;
    transaction.execute(
        "UPDATE workflow_runs SET updated_at = ?2, is_deleted = 1
         WHERE workspace_id = ?1 AND is_deleted = 0",
        params![workspace_id, deleted_at],
    )?;
    transaction.execute(
        "UPDATE sessions SET updated_at = ?2, is_deleted = 1
         WHERE workspace_id = ?1 AND is_deleted = 0",
        params![workspace_id, deleted_at],
    )?;
    transaction.execute(
        "UPDATE tasks SET updated_at = ?2, is_deleted = 1
         WHERE workspace_id = ?1 AND is_deleted = 0",
        params![workspace_id, deleted_at],
    )?;
    transaction.execute(
        "UPDATE worktrees SET updated_at = ?2, is_deleted = 1
         WHERE workspace_id = ?1 AND is_deleted = 0",
        params![workspace_id, deleted_at],
    )?;
    transaction.execute(
        "UPDATE workspace_provisioning SET state = 'destroying', updated_at = ?2
         WHERE workspace_id = ?1 AND state NOT IN ('destroyed')",
        params![workspace_id, deleted_at],
    )?;
    transaction.execute(
        "UPDATE workspaces SET lifecycle = 'retiring', updated_at = ?2, is_deleted = 1
         WHERE id = ?1 AND is_deleted = 0",
        params![workspace_id, deleted_at],
    )?;
    Ok(())
}

/// Collects cleanup jobs from worktree rows while their workspace and location evidence is visible.
pub(super) fn collect_workspace_cleanup_jobs(
    transaction: &Transaction<'_>,
    workspace_filter: &str,
    filter_value: &str,
    now: i64,
) -> Result<Vec<GitCleanupJob>, rusqlite::Error> {
    let mut statement = transaction.prepare(&format!(
        "SELECT w.id, w.project_id,
                json_extract(main_location.locator_json, '$.path'),
                json_extract(worktree_location.locator_json, '$.path'),
                wt.branch_name
         FROM workspaces w
         JOIN projects p ON p.id = w.project_id AND p.is_deleted = 0
         JOIN worktrees wt ON wt.workspace_id = w.id AND wt.is_deleted = 0
         JOIN workspaces main_workspace
           ON main_workspace.project_id = w.project_id
          AND main_workspace.workspace_kind = 'main'
          AND main_workspace.is_deleted = 0
         JOIN workspace_locations main_location ON main_location.id = main_workspace.location_id
         JOIN workspace_locations worktree_location
           ON worktree_location.id = w.location_id
          AND worktree_location.location_kind = 'local_filesystem'
         WHERE {workspace_filter} AND w.is_deleted = 0 AND wt.branch_name IS NOT NULL"
    ))?;
    let mut rows = statement.query(params![filter_value])?;
    let mut jobs = Vec::new();
    while let Some(row) = rows.next()? {
        jobs.push(GitCleanupJob::pending(
            GitCleanupJobId::new(Uuid::new_v4().to_string()),
            ProjectId::new(row.get::<_, String>(1)?),
            WorkspaceId::new(row.get::<_, String>(0)?),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            now,
        ));
    }
    Ok(jobs)
}

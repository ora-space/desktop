use ora_domain::{ProjectId, SessionStatus, TaskId};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::path::PathBuf;

use crate::repository::RepositoryPool;

/// Identifies one Ora-owned Git checkout that the backend should clean up after commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCleanupTarget {
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub repository_root: PathBuf,
    pub branch_name: String,
}

/// Reports the atomic outcome of an aggregate deletion and any post-commit Git work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CascadeDeleteOutcome {
    Deleted {
        git_cleanup_targets: Vec<GitCleanupTarget>,
    },
    NotFound,
    ActiveSession,
}

/// Performs aggregate soft deletes while returning, but never executing, external cleanup work.
#[derive(Clone, Debug)]
pub struct SqliteCascadeRepository {
    pool: RepositoryPool,
}

impl SqliteCascadeRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Soft-deletes one task, its stopped sessions, and its worktree record atomically.
    pub fn delete_task(
        &self,
        task_id: &TaskId,
        deleted_at: i64,
    ) -> Result<CascadeDeleteOutcome, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            // Acquiring the writer reservation before checking status prevents a load from
            // making a descendant Running between validation and the cascade updates.
            let transaction =
                Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM tasks WHERE id = ?1 AND is_deleted = 0",
                    params![task_id.as_ref()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(CascadeDeleteOutcome::NotFound);
            }
            let running = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sessions
                    WHERE task_id = ?1 AND status = ?2 AND is_deleted = 0
                )",
                params![task_id.as_ref(), SessionStatus::Running.database_value()],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if running {
                return Ok(CascadeDeleteOutcome::ActiveSession);
            }
            let git_cleanup_targets = task_git_cleanup_targets(&transaction, task_id)?;
            transaction.execute(
                "UPDATE sessions SET updated_at = ?2, is_deleted = 1 WHERE task_id = ?1 AND is_deleted = 0",
                params![task_id.as_ref(), deleted_at],
            )?;
            transaction.execute(
                "UPDATE worktrees SET updated_at = ?2, is_deleted = 1 WHERE task_id = ?1 AND is_deleted = 0",
                params![task_id.as_ref(), deleted_at],
            )?;
            transaction.execute(
                "UPDATE tasks SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![task_id.as_ref(), deleted_at],
            )?;
            transaction.commit()?;
            Ok(CascadeDeleteOutcome::Deleted {
                git_cleanup_targets,
            })
        })
    }

    /// Soft-deletes a project aggregate atomically after verifying every session is stopped.
    pub fn delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<CascadeDeleteOutcome, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            // Project deletion needs the same write reservation across every descendant check.
            let transaction =
                Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
            let repository_root = transaction
                .query_row(
                    "SELECT root_path FROM projects WHERE id = ?1 AND is_deleted = 0",
                    params![project_id.as_ref()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(repository_root) = repository_root else {
                return Ok(CascadeDeleteOutcome::NotFound);
            };
            let running = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sessions s
                    JOIN tasks t ON t.id = s.task_id
                    WHERE t.project_id = ?1 AND t.is_deleted = 0
                      AND s.status = ?2 AND s.is_deleted = 0
                )",
                params![project_id.as_ref(), SessionStatus::Running.database_value()],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if running {
                return Ok(CascadeDeleteOutcome::ActiveSession);
            }
            let git_cleanup_targets = project_git_cleanup_targets(
                &transaction,
                project_id,
                PathBuf::from(repository_root),
            )?;
            transaction.execute(
                "UPDATE sessions SET updated_at = ?2, is_deleted = 1
                 WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1 AND is_deleted = 0)
                   AND is_deleted = 0",
                params![project_id.as_ref(), deleted_at],
            )?;
            transaction.execute(
                "UPDATE worktrees SET updated_at = ?2, is_deleted = 1
                 WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?1 AND is_deleted = 0)
                   AND is_deleted = 0",
                params![project_id.as_ref(), deleted_at],
            )?;
            transaction.execute(
                "UPDATE tasks SET updated_at = ?2, is_deleted = 1 WHERE project_id = ?1 AND is_deleted = 0",
                params![project_id.as_ref(), deleted_at],
            )?;
            // Work contexts are renewable leases rather than durable user records, so removing
            // them is the only meaningful cascade operation for this table.
            transaction.execute(
                "DELETE FROM project_work_contexts WHERE project_id = ?1",
                params![project_id.as_ref()],
            )?;
            transaction.execute(
                "UPDATE projects SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![project_id.as_ref(), deleted_at],
            )?;
            transaction.commit()?;
            Ok(CascadeDeleteOutcome::Deleted {
                git_cleanup_targets,
            })
        })
    }
}

/// Captures the linked worktree selected by one task before its records become hidden.
fn task_git_cleanup_targets(
    transaction: &Transaction<'_>,
    task_id: &TaskId,
) -> Result<Vec<GitCleanupTarget>, rusqlite::Error> {
    transaction
        .query_row(
            "SELECT t.project_id, p.root_path, w.branch_name
             FROM tasks t
             JOIN projects p ON p.id = t.project_id
             JOIN worktrees w ON w.id = t.worktree_id AND w.task_id = t.id
             WHERE t.id = ?1 AND t.is_deleted = 0
               AND w.is_deleted = 0 AND w.branch_name IS NOT NULL",
            params![task_id.as_ref()],
            |row| {
                Ok(GitCleanupTarget {
                    project_id: ProjectId::new(row.get::<_, String>(0)?),
                    task_id: task_id.clone(),
                    repository_root: PathBuf::from(row.get::<_, String>(1)?),
                    branch_name: row.get(2)?,
                })
            },
        )
        .optional()
        .map(|target| target.into_iter().collect())
}

/// Captures every linked worktree selected by the visible tasks in one project.
fn project_git_cleanup_targets(
    transaction: &Transaction<'_>,
    project_id: &ProjectId,
    repository_root: PathBuf,
) -> Result<Vec<GitCleanupTarget>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT t.id, w.branch_name
         FROM tasks t
         JOIN worktrees w ON w.id = t.worktree_id AND w.task_id = t.id
         WHERE t.project_id = ?1 AND t.is_deleted = 0
           AND w.is_deleted = 0 AND w.branch_name IS NOT NULL
         ORDER BY t.created_at, t.id",
    )?;
    let targets = statement.query_map(params![project_id.as_ref()], |row| {
        Ok(GitCleanupTarget {
            project_id: project_id.clone(),
            task_id: TaskId::new(row.get::<_, String>(0)?),
            repository_root: repository_root.clone(),
            branch_name: row.get(1)?,
        })
    })?;

    targets.collect()
}

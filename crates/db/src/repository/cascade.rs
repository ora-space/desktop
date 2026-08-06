use ora_domain::{ProjectId, SessionStatus, TaskId};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::repository::RepositoryPool;

/// Reports the atomic outcome of an Ora-only aggregate deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeDeleteOutcome {
    Deleted,
    NotFound,
    ActiveSession,
}

/// Performs aggregate soft deletes in one SQLite transaction without invoking Git.
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
            Ok(CascadeDeleteOutcome::Deleted)
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
            transaction.execute(
                "UPDATE project_spec_source_overrides
                 SET updated_at = ?2, is_deleted = 1
                 WHERE project_id = ?1 AND is_deleted = 0",
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
            Ok(CascadeDeleteOutcome::Deleted)
        })
    }
}

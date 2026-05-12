use ora_application::{WorktreeRepository, WorktreeRepositoryError};
use ora_domain::{AuditFields, TaskId, Worktree, WorktreeActivity, WorktreeId};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists worktree snapshots through SQLite while hiding storage details from handlers.
#[derive(Clone, Debug)]
pub struct SqliteWorktreeRepository {
    pool: RepositoryPool,
}

impl SqliteWorktreeRepository {
    /// Builds a worktree repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl WorktreeRepository for SqliteWorktreeRepository {
    /// Inserts a new worktree row and returns the stored worktree snapshot.
    fn create_worktree(&self, worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO worktrees (id, task_id, branch_name, is_active, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        worktree.id.as_ref(),
                        worktree.task_id.as_ref(),
                        worktree.branch_name.as_deref(),
                        worktree.activity.database_value(),
                        worktree.audit_fields.created_at,
                        worktree.audit_fields.updated_at,
                        bool_to_sqlite(worktree.audit_fields.is_deleted),
                    ],
                )?;

                Ok(worktree)
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Loads one visible worktree row by identifier.
    fn find_worktree(
        &self,
        worktree_id: &WorktreeId,
    ) -> Result<Option<Worktree>, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, task_id, branch_name, is_active, created_at, updated_at, is_deleted
                     FROM worktrees
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![worktree_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_worktree_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Lists every visible worktree row in stable storage order.
    fn list_worktrees(&self) -> Result<Vec<Worktree>, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, task_id, branch_name, is_active, created_at, updated_at, is_deleted
                     FROM worktrees
                     WHERE is_deleted = 0
                     ORDER BY created_at, id",
                )?;
                let mut rows = statement.query([])?;
                let mut worktrees = Vec::new();

                while let Some(row) = rows.next()? {
                    worktrees.push(map_worktree_row(row)?);
                }

                Ok(worktrees)
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Replaces the persisted worktree snapshot identified by the provided id.
    fn update_worktree(&self, worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE worktrees
                     SET task_id = ?2, branch_name = ?3, is_active = ?4, created_at = ?5, updated_at = ?6, is_deleted = ?7
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        worktree.id.as_ref(),
                        worktree.task_id.as_ref(),
                        worktree.branch_name.as_deref(),
                        worktree.activity.database_value(),
                        worktree.audit_fields.created_at,
                        worktree.audit_fields.updated_at,
                        bool_to_sqlite(worktree.audit_fields.is_deleted),
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::QueryReturnedNoRows));
                }

                Ok(worktree)
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Soft-deletes one visible worktree row and reports whether it existed.
    fn soft_delete_worktree(
        &self,
        worktree_id: &WorktreeId,
        deleted_at: i64,
    ) -> Result<bool, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE worktrees
                     SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![worktree_id.as_ref(), deleted_at],
                )?;

                Ok(updated_rows > 0)
            })
            .map_err(worktree_repository_error_from_database)
    }
}

/// Reconstructs a domain worktree from the selected worktree columns.
fn map_worktree_row(row: &Row<'_>) -> Result<Worktree, crate::DatabaseError> {
    let activity = WorktreeActivity::from_database_value(row.get("is_active")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(Worktree::new(
        WorktreeId::new(row.get::<_, String>("id")?),
        TaskId::new(row.get::<_, String>("task_id")?),
        row.get::<_, Option<String>>("branch_name")?,
        activity,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Converts shared database-layer failures into worktree repository errors.
fn worktree_repository_error_from_database(error: crate::DatabaseError) -> WorktreeRepositoryError {
    WorktreeRepositoryError::OperationFailed(error.to_string())
}

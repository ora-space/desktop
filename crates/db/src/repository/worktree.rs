use ora_application::{RepositoryError, WorktreeRepository};
use ora_domain::{AuditFields, WorkspaceId, Worktree, WorktreeActivity, WorktreeBaseline};
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
    fn create_worktree(&self, worktree: Worktree) -> Result<Worktree, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO worktrees (workspace_id, branch_name, base_commit_id, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        worktree.workspace_id.as_ref(),
                        worktree.branch_name.as_deref(),
                        baseline_value(&worktree.baseline),
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
        workspace_id: &WorkspaceId,
    ) -> Result<Option<Worktree>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT workspace_id, branch_name, base_commit_id, created_at, updated_at, is_deleted
                     FROM worktrees
                     WHERE workspace_id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![workspace_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_worktree_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Lists every visible worktree row in stable storage order.
    fn list_worktrees(&self) -> Result<Vec<Worktree>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT workspace_id, branch_name, base_commit_id, created_at, updated_at, is_deleted
                     FROM worktrees
                     WHERE is_deleted = 0
                     ORDER BY created_at, workspace_id",
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
    fn update_worktree(&self, worktree: Worktree) -> Result<Worktree, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE worktrees
                     SET branch_name = ?2, base_commit_id = ?3, created_at = ?4, updated_at = ?5, is_deleted = ?6
                     WHERE workspace_id = ?1 AND is_deleted = 0",
                    params![
                        worktree.workspace_id.as_ref(),
                        worktree.branch_name.as_deref(),
                        baseline_value(&worktree.baseline),
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
        workspace_id: &WorkspaceId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE worktrees
                     SET updated_at = ?2, is_deleted = 1
                     WHERE workspace_id = ?1 AND is_deleted = 0",
                    params![workspace_id.as_ref(), deleted_at],
                )?;

                Ok(updated_rows > 0)
            })
            .map_err(worktree_repository_error_from_database)
    }
}

/// Reconstructs a domain worktree from the selected worktree columns.
pub(super) fn map_worktree_row(row: &Row<'_>) -> Result<Worktree, crate::DatabaseError> {
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(Worktree::new(
        WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        row.get::<_, Option<String>>("branch_name")?,
        match row.get::<_, Option<String>>("base_commit_id")? {
            Some(commit_id) => WorktreeBaseline::recorded(commit_id)?,
            None => WorktreeBaseline::unavailable(),
        },
        WorktreeActivity::Active,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Maps the explicit domain baseline state into the nullable migration representation.
fn baseline_value(baseline: &WorktreeBaseline) -> Option<&str> {
    baseline.commit_id()
}

/// Converts shared database-layer failures into worktree repository errors.
fn worktree_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

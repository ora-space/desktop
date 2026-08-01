use ora_application::{ProjectWorkContextRepository, RepositoryError};
use ora_domain::{ProjectId, ProjectWorkContext, ProjectWorkContextId, ProjectWorkContextSurface};
use rusqlite::{Row, params};

use crate::repository::RepositoryPool;

/// Persists project work context snapshots through SQLite while hiding storage details from handlers.
#[derive(Clone, Debug)]
pub struct SqliteProjectWorkContextRepository {
    pool: RepositoryPool,
}

impl SqliteProjectWorkContextRepository {
    /// Builds a project work context repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl ProjectWorkContextRepository for SqliteProjectWorkContextRepository {
    /// Inserts a new project work context row and returns the stored snapshot.
    fn create_project_work_context(
        &self,
        context: ProjectWorkContext,
    ) -> Result<ProjectWorkContext, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO project_work_contexts (
                        id,
                        surface,
                        window_id,
                        project_id,
                        lease_expires_at,
                        created_at,
                        updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        context.id.as_ref(),
                        context.surface.database_value(),
                        &context.window_id,
                        context.project_id.as_ref(),
                        context.lease_expires_at,
                        context.created_at,
                        context.updated_at,
                    ],
                )?;

                Ok(context)
            })
            .map_err(project_work_context_repository_error_from_database)
    }

    /// Loads one work context row by surface and window identity.
    fn find_project_work_context(
        &self,
        surface: ProjectWorkContextSurface,
        window_id: &str,
    ) -> Result<Option<ProjectWorkContext>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, surface, window_id, project_id, lease_expires_at, created_at, updated_at
                     FROM project_work_contexts
                     WHERE surface = ?1 AND window_id = ?2",
                )?;
                let mut rows = statement.query(params![surface.database_value(), window_id])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_project_work_context_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(project_work_context_repository_error_from_database)
    }

    /// Loads one active work context for the requested project using backend expiry time.
    fn find_active_project_work_context_for_project(
        &self,
        project_id: &ProjectId,
        active_after: i64,
    ) -> Result<Option<ProjectWorkContext>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, surface, window_id, project_id, lease_expires_at, created_at, updated_at
                     FROM project_work_contexts
                     WHERE project_id = ?1 AND lease_expires_at > ?2
                     ORDER BY lease_expires_at DESC, updated_at DESC, id
                     LIMIT 1",
                )?;
                let mut rows = statement.query(params![project_id.as_ref(), active_after])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_project_work_context_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(project_work_context_repository_error_from_database)
    }

    /// Replaces the persisted work context snapshot identified by the provided id.
    fn update_project_work_context(
        &self,
        context: ProjectWorkContext,
    ) -> Result<ProjectWorkContext, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE project_work_contexts
                     SET surface = ?2,
                         window_id = ?3,
                         project_id = ?4,
                         lease_expires_at = ?5,
                         created_at = ?6,
                         updated_at = ?7
                     WHERE id = ?1",
                    params![
                        context.id.as_ref(),
                        context.surface.database_value(),
                        &context.window_id,
                        context.project_id.as_ref(),
                        context.lease_expires_at,
                        context.created_at,
                        context.updated_at,
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    ));
                }

                Ok(context)
            })
            .map_err(project_work_context_repository_error_from_database)
    }

    /// Deletes one work context row by surface and window identity and reports whether it existed.
    fn delete_project_work_context(
        &self,
        surface: ProjectWorkContextSurface,
        window_id: &str,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let deleted_rows = connection.execute(
                    "DELETE FROM project_work_contexts
                     WHERE surface = ?1 AND window_id = ?2",
                    params![surface.database_value(), window_id],
                )?;

                Ok(deleted_rows > 0)
            })
            .map_err(project_work_context_repository_error_from_database)
    }

    /// Deletes expired rows older than the supplied retention cutoff and returns the row count.
    fn delete_expired_project_work_contexts(
        &self,
        expired_before: i64,
    ) -> Result<usize, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let deleted_rows = connection.execute(
                    "DELETE FROM project_work_contexts
                     WHERE lease_expires_at < ?1",
                    params![expired_before],
                )?;

                Ok(deleted_rows)
            })
            .map_err(project_work_context_repository_error_from_database)
    }
}

/// Reconstructs a domain project work context from the selected columns.
fn map_project_work_context_row(row: &Row<'_>) -> Result<ProjectWorkContext, crate::DatabaseError> {
    let surface_value = row.get::<_, String>("surface")?;

    Ok(ProjectWorkContext::new(
        ProjectWorkContextId::new(row.get::<_, String>("id")?),
        ProjectWorkContextSurface::from_database_value(&surface_value)?,
        row.get::<_, String>("window_id")?,
        ProjectId::new(row.get::<_, String>("project_id")?),
        row.get("lease_expires_at")?,
        row.get("created_at")?,
        row.get("updated_at")?,
    ))
}

/// Converts shared database-layer failures into project work context repository errors.
fn project_work_context_repository_error_from_database(
    error: crate::DatabaseError,
) -> RepositoryError {
    RepositoryError::new(error)
}

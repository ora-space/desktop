use ora_application::{ProjectRepository, ProjectRepositoryError};
use ora_domain::{AuditFields, Project, ProjectId};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists project snapshots through SQLite while hiding storage details from handlers.
#[derive(Clone, Debug)]
pub struct SqliteProjectRepository {
    pool: RepositoryPool,
}

impl SqliteProjectRepository {
    /// Builds a project repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl ProjectRepository for SqliteProjectRepository {
    /// Inserts a new project row and returns the stored project snapshot.
    fn create_project(&self, project: Project) -> Result<Project, ProjectRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO projects (id, name, root_path, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        project.id.as_ref(),
                        &project.name,
                        &project.root_path,
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                        bool_to_sqlite(project.audit_fields.is_deleted),
                    ],
                )?;

                Ok(project)
            })
            .map_err(project_repository_error_from_database)
    }

    /// Loads one visible project row by identifier.
    fn find_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, name, root_path, created_at, updated_at, is_deleted
                     FROM projects
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![project_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_project_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(project_repository_error_from_database)
    }

    /// Lists every visible project row in stable storage order.
    fn list_projects(&self) -> Result<Vec<Project>, ProjectRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, name, root_path, created_at, updated_at, is_deleted
                     FROM projects
                     WHERE is_deleted = 0
                     ORDER BY created_at, id",
                )?;
                let mut rows = statement.query([])?;
                let mut projects = Vec::new();

                while let Some(row) = rows.next()? {
                    projects.push(map_project_row(row)?);
                }

                Ok(projects)
            })
            .map_err(project_repository_error_from_database)
    }

    /// Replaces the persisted project snapshot identified by the provided id.
    fn update_project(&self, project: Project) -> Result<Project, ProjectRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE projects
                     SET name = ?2, root_path = ?3, created_at = ?4, updated_at = ?5, is_deleted = ?6
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        project.id.as_ref(),
                        &project.name,
                        &project.root_path,
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                        bool_to_sqlite(project.audit_fields.is_deleted),
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::QueryReturnedNoRows));
                }

                Ok(project)
            })
            .map_err(project_repository_error_from_database)
    }

    /// Soft-deletes one visible project row and reports whether it existed.
    fn soft_delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<bool, ProjectRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE projects
                     SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![project_id.as_ref(), deleted_at],
                )?;

                Ok(updated_rows > 0)
            })
            .map_err(project_repository_error_from_database)
    }
}

/// Reconstructs a domain project from the selected project columns.
fn map_project_row(row: &Row<'_>) -> Result<Project, crate::DatabaseError> {
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(Project::new(
        ProjectId::new(row.get::<_, String>("id")?),
        row.get::<_, String>("name")?,
        row.get::<_, String>("root_path")?,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Converts shared database-layer failures into project repository errors.
fn project_repository_error_from_database(error: crate::DatabaseError) -> ProjectRepositoryError {
    ProjectRepositoryError::OperationFailed(error.to_string())
}

use ora_application::{ProjectRepository, RepositoryError};
use ora_domain::{AuditFields, Project, ProjectId, WorkspaceLocation};
use rusqlite::{Row, Transaction, TransactionBehavior, params};
use std::path::Path;
use uuid::Uuid;

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
    fn create_project(
        &self,
        project: Project,
        main_workspace_location: WorkspaceLocation,
    ) -> Result<Project, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                transaction.execute(
                    "INSERT INTO projects (id, name, repository_kind, repository_url, default_branch, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        project.id.as_ref(),
                        &project.name,
                        &project.repository_kind,
                        project.repository_url.as_deref(),
                        project.default_branch.as_deref(),
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                        bool_to_sqlite(project.audit_fields.is_deleted),
                    ],
                )?;

                let location_id = Uuid::new_v4().to_string();
                let workspace_id = Uuid::new_v4().to_string();
                let (location_kind, plugin_id, locator) = encode_location(&main_workspace_location);
                let (workspace_lifecycle, provisioning_state, provisioner_kind, provisioner_plugin) =
                    initial_workspace_state(&main_workspace_location);
                transaction.execute(
                    "INSERT INTO workspace_locations (id, location_kind, plugin_id, locator_version, locator_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)",
                    params![
                        location_id,
                        location_kind,
                        plugin_id,
                        locator,
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, 'main', ?3, ?4, ?5, ?6, ?7)",
                    params![
                        workspace_id,
                        project.id.as_ref(),
                        location_id,
                        workspace_lifecycle,
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                        bool_to_sqlite(project.audit_fields.is_deleted),
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO workspace_provisioning (workspace_id, provisioner_kind, plugin_id, state, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        workspace_id,
                        provisioner_kind,
                        provisioner_plugin,
                        provisioning_state,
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                    ],
                )?;
                transaction.commit()?;

                Ok(project)
            })
            .map_err(project_repository_error_from_database)
    }

    /// Loads one visible project row by identifier.
    fn find_project(&self, project_id: &ProjectId) -> Result<Option<Project>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT p.id, p.name, p.repository_kind, p.repository_url, p.default_branch,
                            p.created_at, p.updated_at, p.is_deleted
                     FROM projects p
                     WHERE p.id = ?1 AND p.is_deleted = 0",
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
    fn list_projects(&self) -> Result<Vec<Project>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT p.id, p.name, p.repository_kind, p.repository_url, p.default_branch,
                            p.created_at, p.updated_at, p.is_deleted
                     FROM projects p
                     WHERE p.is_deleted = 0
                     ORDER BY p.created_at, p.id",
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
    fn update_project(&self, project: Project) -> Result<Project, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                let updated_rows = transaction.execute(
                    "UPDATE projects
                     SET name = ?2, repository_kind = ?3, repository_url = ?4, default_branch = ?5,
                         created_at = ?6, updated_at = ?7, is_deleted = ?8
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        project.id.as_ref(),
                        &project.name,
                        &project.repository_kind,
                        project.repository_url.as_deref(),
                        project.default_branch.as_deref(),
                        project.audit_fields.created_at,
                        project.audit_fields.updated_at,
                        bool_to_sqlite(project.audit_fields.is_deleted),
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    ));
                }

                transaction.commit()?;

                Ok(project)
            })
            .map_err(project_repository_error_from_database)
    }

    /// Soft-deletes one visible project row and reports whether it existed.
    fn soft_delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
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
    let mut project = Project::new(
        ProjectId::new(row.get::<_, String>("id")?),
        row.get::<_, String>("name")?,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    );
    project.repository_kind = row.get("repository_kind")?;
    project.repository_url = row.get("repository_url")?;
    project.default_branch = row.get("default_branch")?;
    Ok(project)
}

/// Encodes a tagged workspace location at the database boundary without leaking its opaque shape.
fn encode_location(location: &WorkspaceLocation) -> (&'static str, Option<&str>, String) {
    match location {
        WorkspaceLocation::LocalFilesystem { path } => (
            "local_filesystem",
            None,
            serde_json::json!({ "path": path }).to_string(),
        ),
        WorkspaceLocation::Ssh {
            connection_ref,
            path,
        } => (
            "ssh",
            None,
            serde_json::json!({ "connection_ref": connection_ref, "path": path }).to_string(),
        ),
        WorkspaceLocation::RemoteTarget {
            plugin_id,
            target_ref,
            locator,
        } => (
            "remote_target",
            Some(plugin_id),
            serde_json::json!({ "target_ref": target_ref, "locator": locator }).to_string(),
        ),
    }
}

/// Determines whether a main Workspace already has a local location that can be admitted.
fn initial_workspace_state(
    location: &WorkspaceLocation,
) -> (&'static str, &'static str, &'static str, Option<&str>) {
    match location {
        WorkspaceLocation::LocalFilesystem { path } => {
            let ready = Path::new(path).is_dir();
            if ready {
                ("active", "ready", "local_git", None)
            } else {
                ("provisioning", "pending", "local_git", None)
            }
        }
        WorkspaceLocation::Ssh { .. } => ("provisioning", "pending", "ssh", None),
        WorkspaceLocation::RemoteTarget { plugin_id, .. } => {
            ("provisioning", "pending", "remote_target", Some(plugin_id))
        }
    }
}

/// Converts shared database-layer failures into project repository errors.
fn project_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

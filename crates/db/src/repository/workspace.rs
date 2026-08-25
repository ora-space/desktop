use ora_application::WorkspaceRepository;
use ora_domain::{
    AuditFields, ProjectId, Workspace, WorkspaceId, WorkspaceKind, WorkspaceLifecycle,
    WorkspaceLocation,
};
use rusqlite::{OptionalExtension, Row, params};

use crate::repository::RepositoryPool;

/// Reads workspace identity and location records without exposing SQLite locator encoding.
#[derive(Clone, Debug)]
pub struct SqliteWorkspaceRepository {
    pool: RepositoryPool,
}

impl SqliteWorkspaceRepository {
    /// Builds a workspace repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Loads one visible workspace and rejects unknown lifecycle or location values explicitly.
    pub fn find_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<Workspace>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{} WHERE w.id = ?1 AND w.is_deleted = 0",
                workspace_select_sql()
            ))?;
            let mut rows = statement.query(params![workspace_id.as_ref()])?;
            match rows.next()? {
                Some(row) => Ok(Some(map_workspace_row(row)?)),
                None => Ok(None),
            }
        })
    }

    /// Finds the active main workspace selected by the project's unique main-workspace index.
    pub fn find_main_workspace(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Workspace>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{} WHERE w.project_id = ?1 AND w.workspace_kind = 'main' AND w.is_deleted = 0",
                workspace_select_sql()
            ))?;
            let mut rows = statement.query(params![project_id.as_ref()])?;
            match rows.next()? {
                Some(row) => Ok(Some(map_workspace_row(row)?)),
                None => Ok(None),
            }
        })
    }

    /// Lists visible workspaces belonging to a project in stable creation order.
    pub fn list_workspaces(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Workspace>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{} WHERE w.project_id = ?1 AND w.is_deleted = 0 ORDER BY w.created_at, w.id",
                workspace_select_sql()
            ))?;
            let mut rows = statement.query(params![project_id.as_ref()])?;
            let mut workspaces = Vec::new();
            while let Some(row) = rows.next()? {
                workspaces.push(map_workspace_row(row)?);
            }
            Ok(workspaces)
        })
    }

    /// Lists every visible workspace so adapters can resolve project ownership without Task rows.
    pub fn list_all_workspaces(&self) -> Result<Vec<Workspace>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(&format!(
                "{} WHERE w.is_deleted = 0 ORDER BY w.created_at, w.id",
                workspace_select_sql()
            ))?;
            let mut rows = statement.query([])?;
            let mut workspaces = Vec::new();
            while let Some(row) = rows.next()? {
                workspaces.push(map_workspace_row(row)?);
            }
            Ok(workspaces)
        })
    }

    /// Resolves the workspace behind a user-facing worktree task projection.
    pub fn find_workspace_for_task(
        &self,
        task_id: &ora_domain::TaskId,
    ) -> Result<Option<Workspace>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let workspace_id = connection
                .query_row(
                    "SELECT workspace_id FROM tasks WHERE id = ?1 AND is_deleted = 0",
                    params![task_id.as_ref()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            workspace_id
                .map(|id| {
                    let mut statement = connection.prepare(workspace_select_sql())?;
                    let mut rows = statement.query(params![id])?;
                    match rows.next()? {
                        Some(row) => map_workspace_row(row).map(Some),
                        None => Ok(None),
                    }
                })
                .transpose()
                .map(Option::flatten)
        })
    }
}

impl WorkspaceRepository for SqliteWorkspaceRepository {
    /// Loads a workspace through the application admission port.
    fn find_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<Workspace>, ora_application::RepositoryError> {
        SqliteWorkspaceRepository::find_workspace(self, workspace_id)
            .map_err(ora_application::RepositoryError::new)
    }

    /// Loads the project's canonical main workspace through the application admission port.
    fn find_main_workspace(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Workspace>, ora_application::RepositoryError> {
        SqliteWorkspaceRepository::find_main_workspace(self, project_id)
            .map_err(ora_application::RepositoryError::new)
    }

    /// Reads the durable provisioning state without exposing its storage representation.
    fn is_provisioning_ready(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<bool, ora_application::RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let state = connection
                    .query_row(
                        "SELECT state FROM workspace_provisioning WHERE workspace_id = ?1",
                        rusqlite::params![workspace_id.as_ref()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                match state {
                    Some(state) => Ok(ora_domain::WorkspaceProvisioningState::from_database_value(
                        &state,
                    )? == ora_domain::WorkspaceProvisioningState::Ready),
                    None => Ok(false),
                }
            })
            .map_err(ora_application::RepositoryError::new)
    }
}

/// Returns the shared workspace projection used by all lookup methods.
pub(super) fn workspace_select_sql() -> &'static str {
    "SELECT w.id, w.project_id, w.workspace_kind, w.lifecycle,
            w.created_at, w.updated_at, w.is_deleted,
            l.location_kind, l.plugin_id, l.locator_json
     FROM workspaces w
     JOIN projects p ON p.id = w.project_id AND p.is_deleted = 0
     JOIN workspace_locations l ON l.id = w.location_id"
}

/// Reconstructs a workspace and its tagged location from database columns.
pub(super) fn map_workspace_row(row: &Row<'_>) -> Result<Workspace, crate::DatabaseError> {
    let location_kind = row.get::<_, String>("location_kind")?;
    let locator_json = row.get::<_, String>("locator_json")?;
    let locator = serde_json::from_str::<serde_json::Value>(&locator_json)
        .map_err(crate::DatabaseError::CorruptWorkflowRunState)?;
    let location = match location_kind.as_str() {
        "local_filesystem" => WorkspaceLocation::LocalFilesystem {
            path: required_locator_string(&locator, "path")?,
        },
        "ssh" => WorkspaceLocation::Ssh {
            connection_ref: required_locator_string(&locator, "connection_ref")?,
            path: required_locator_string(&locator, "path")?,
        },
        "remote_target" => WorkspaceLocation::RemoteTarget {
            plugin_id: row.get::<_, Option<String>>("plugin_id")?.ok_or_else(|| {
                crate::DatabaseError::DomainModel(
                    ora_domain::DomainModelError::InvalidWorkspaceLocationKind(
                        "remote_target without plugin id".to_string(),
                    ),
                )
            })?,
            target_ref: required_locator_string(&locator, "target_ref")?,
            locator: required_locator_string(&locator, "locator")?,
        },
        other => {
            return Err(crate::DatabaseError::DomainModel(
                ora_domain::DomainModelError::InvalidWorkspaceLocationKind(other.to_string()),
            ));
        }
    };
    Ok(Workspace::new(
        WorkspaceId::new(row.get::<_, String>("id")?),
        ProjectId::new(row.get::<_, String>("project_id")?),
        WorkspaceKind::from_database_value(row.get::<_, String>("workspace_kind")?.as_str())?,
        location,
        WorkspaceLifecycle::from_database_value(row.get::<_, String>("lifecycle")?.as_str())?,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    ))
}

/// Reads one required string from an opaque locator object.
fn required_locator_string(
    locator: &serde_json::Value,
    key: &'static str,
) -> Result<String, crate::DatabaseError> {
    locator
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            crate::DatabaseError::DomainModel(
                ora_domain::DomainModelError::InvalidWorkspaceLocationKind(format!(
                    "workspace locator is missing {key}"
                )),
            )
        })
}

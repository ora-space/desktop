use ora_application::{RepositoryError, TaskRepository};
use ora_domain::{AuditFields, ProjectId, Task, TaskId, WorkspaceId};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists task snapshots through SQLite while hiding storage details from handlers.
#[derive(Clone, Debug)]
pub struct SqliteTaskRepository {
    pool: RepositoryPool,
}

impl SqliteTaskRepository {
    /// Builds a task repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl TaskRepository for SqliteTaskRepository {
    /// Inserts a new task row and returns the stored task snapshot.
    fn create_task(&self, task: Task) -> Result<Task, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO tasks (id, workspace_id, title, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        task.id.as_ref(),
                        task.workspace_id.as_ref(),
                        &task.title,
                        task.audit_fields.created_at,
                        task.audit_fields.updated_at,
                        bool_to_sqlite(task.audit_fields.is_deleted),
                    ],
                )?;

                Ok(task)
            })
            .map_err(task_repository_error_from_database)
    }

    /// Loads one visible task row by identifier.
    fn find_task(&self, task_id: &TaskId) -> Result<Option<Task>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT t.id, w.project_id, t.workspace_id, t.title,
                            t.created_at, t.updated_at, t.is_deleted
                     FROM tasks t
                     JOIN workspaces w ON w.id = t.workspace_id
                     WHERE t.id = ?1 AND t.is_deleted = 0",
                )?;
                let mut rows = statement.query(params![task_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_task_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(task_repository_error_from_database)
    }

    /// Lists every visible task row in stable storage order.
    fn list_tasks(&self) -> Result<Vec<Task>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT t.id, w.project_id, t.workspace_id, t.title,
                            t.created_at, t.updated_at, t.is_deleted
                     FROM tasks t
                     JOIN workspaces w ON w.id = t.workspace_id
                     WHERE t.is_deleted = 0
                     ORDER BY t.created_at, t.id",
                )?;
                let mut rows = statement.query([])?;
                let mut tasks = Vec::new();

                while let Some(row) = rows.next()? {
                    tasks.push(map_task_row(row)?);
                }

                Ok(tasks)
            })
            .map_err(task_repository_error_from_database)
    }

    /// Replaces the persisted task snapshot identified by the provided id.
    fn update_task(&self, task: Task) -> Result<Task, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE tasks
                     SET workspace_id = ?2, title = ?3, created_at = ?4, updated_at = ?5, is_deleted = ?6
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        task.id.as_ref(),
                        task.workspace_id.as_ref(),
                        &task.title,
                        task.audit_fields.created_at,
                        task.audit_fields.updated_at,
                        bool_to_sqlite(task.audit_fields.is_deleted),
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::QueryReturnedNoRows));
                }

                Ok(task)
            })
            .map_err(task_repository_error_from_database)
    }

    /// Soft-deletes one visible task row and reports whether it existed.
    fn soft_delete_task(&self, task_id: &TaskId, deleted_at: i64) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE tasks
                     SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![task_id.as_ref(), deleted_at],
                )?;

                Ok(updated_rows > 0)
            })
            .map_err(task_repository_error_from_database)
    }
}

/// Reconstructs a domain task from the selected task columns.
pub(super) fn map_task_row(row: &Row<'_>) -> Result<Task, crate::DatabaseError> {
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(Task {
        id: TaskId::new(row.get::<_, String>("id")?),
        project_id: ProjectId::new(row.get::<_, String>("project_id")?),
        workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        title: row.get::<_, String>("title")?,
        audit_fields: AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    })
}

/// Converts shared database-layer failures into task repository errors.
fn task_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

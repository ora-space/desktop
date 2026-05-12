use ora_application::{TaskRepository, TaskRepositoryError};
use ora_domain::{AuditFields, ProjectId, Task, TaskId, TaskStatus, WorktreeId};
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
    fn create_task(&self, task: Task) -> Result<Task, TaskRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO tasks (id, project_id, title, status, worktree_id, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        task.id.as_ref(),
                        task.project_id.as_ref(),
                        &task.title,
                        task.status.database_value(),
                        task.worktree_id.as_ref().map(AsRef::as_ref),
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
    fn find_task(&self, task_id: &TaskId) -> Result<Option<Task>, TaskRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, project_id, title, status, worktree_id, created_at, updated_at, is_deleted
                     FROM tasks
                     WHERE id = ?1 AND is_deleted = 0",
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
    fn list_tasks(&self) -> Result<Vec<Task>, TaskRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, project_id, title, status, worktree_id, created_at, updated_at, is_deleted
                     FROM tasks
                     WHERE is_deleted = 0
                     ORDER BY created_at, id",
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
    fn update_task(&self, task: Task) -> Result<Task, TaskRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE tasks
                     SET project_id = ?2, title = ?3, status = ?4, worktree_id = ?5, created_at = ?6, updated_at = ?7, is_deleted = ?8
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        task.id.as_ref(),
                        task.project_id.as_ref(),
                        &task.title,
                        task.status.database_value(),
                        task.worktree_id.as_ref().map(AsRef::as_ref),
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
    fn soft_delete_task(
        &self,
        task_id: &TaskId,
        deleted_at: i64,
    ) -> Result<bool, TaskRepositoryError> {
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
fn map_task_row(row: &Row<'_>) -> Result<Task, crate::DatabaseError> {
    let worktree_id = row
        .get::<_, Option<String>>("worktree_id")?
        .map(WorktreeId::new);
    let status = TaskStatus::from_database_value(row.get("status")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(Task::new(
        TaskId::new(row.get::<_, String>("id")?),
        ProjectId::new(row.get::<_, String>("project_id")?),
        row.get::<_, String>("title")?,
        status,
        worktree_id,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Converts shared database-layer failures into task repository errors.
fn task_repository_error_from_database(error: crate::DatabaseError) -> TaskRepositoryError {
    TaskRepositoryError::OperationFailed(error.to_string())
}

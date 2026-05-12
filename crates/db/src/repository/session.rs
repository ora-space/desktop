use ora_application::{SessionRepository, SessionRepositoryError};
use ora_domain::{AuditFields, Session, SessionId, SessionStatus, TaskId};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists session snapshots through SQLite while hiding storage details from handlers.
#[derive(Clone, Debug)]
pub struct SqliteSessionRepository {
    pool: RepositoryPool,
}

impl SqliteSessionRepository {
    /// Builds a session repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl SessionRepository for SqliteSessionRepository {
    /// Inserts a new session row and returns the stored session snapshot.
    fn create_session(&self, session: Session) -> Result<Session, SessionRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO sessions (id, task_id, agent_id, agent_session_id, status, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        session.id.as_ref(),
                        session.task_id.as_ref(),
                        &session.agent_id,
                        session.agent_session_id.as_deref(),
                        session.status.database_value(),
                        session.audit_fields.created_at,
                        session.audit_fields.updated_at,
                        bool_to_sqlite(session.audit_fields.is_deleted),
                    ],
                )?;

                Ok(session)
            })
            .map_err(session_repository_error_from_database)
    }

    /// Loads one visible session row by identifier.
    fn find_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, task_id, agent_id, agent_session_id, status, created_at, updated_at, is_deleted
                     FROM sessions
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![session_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_session_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(session_repository_error_from_database)
    }

    /// Lists every visible session row in stable storage order.
    fn list_sessions(&self) -> Result<Vec<Session>, SessionRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, task_id, agent_id, agent_session_id, status, created_at, updated_at, is_deleted
                     FROM sessions
                     WHERE is_deleted = 0
                     ORDER BY created_at, id",
                )?;
                let mut rows = statement.query([])?;
                let mut sessions = Vec::new();

                while let Some(row) = rows.next()? {
                    sessions.push(map_session_row(row)?);
                }

                Ok(sessions)
            })
            .map_err(session_repository_error_from_database)
    }

    /// Replaces the persisted session snapshot identified by the provided id.
    fn update_session(&self, session: Session) -> Result<Session, SessionRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE sessions
                     SET task_id = ?2, agent_id = ?3, agent_session_id = ?4, status = ?5, created_at = ?6, updated_at = ?7, is_deleted = ?8
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        session.id.as_ref(),
                        session.task_id.as_ref(),
                        &session.agent_id,
                        session.agent_session_id.as_deref(),
                        session.status.database_value(),
                        session.audit_fields.created_at,
                        session.audit_fields.updated_at,
                        bool_to_sqlite(session.audit_fields.is_deleted),
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::QueryReturnedNoRows));
                }

                Ok(session)
            })
            .map_err(session_repository_error_from_database)
    }

    /// Soft-deletes one visible session row and reports whether it existed.
    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, SessionRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE sessions
                     SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![session_id.as_ref(), deleted_at],
                )?;

                Ok(updated_rows > 0)
            })
            .map_err(session_repository_error_from_database)
    }
}

/// Reconstructs a domain session from the selected session columns.
fn map_session_row(row: &Row<'_>) -> Result<Session, crate::DatabaseError> {
    let status = SessionStatus::from_database_value(row.get("status")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(Session::new(
        SessionId::new(row.get::<_, String>("id")?),
        TaskId::new(row.get::<_, String>("task_id")?),
        row.get::<_, String>("agent_id")?,
        row.get::<_, Option<String>>("agent_session_id")?,
        status,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Converts shared database-layer failures into session repository errors.
fn session_repository_error_from_database(error: crate::DatabaseError) -> SessionRepositoryError {
    SessionRepositoryError::OperationFailed(error.to_string())
}

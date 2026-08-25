use ora_application::{RepositoryError, SessionRepository};
use ora_domain::{
    AgentRef, AuditFields, DomainModelError, HistoryState, Session, SessionId, SessionStatus,
    SessionTitle, WorkspaceId,
};
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
    fn create_session(&self, session: Session) -> Result<Session, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let inserted_rows = connection.execute(
                    "INSERT INTO sessions (id, workspace_id, agent_cli, agent_session_id, title, status, history_degraded_reason, created_at, updated_at, is_deleted)
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
                     WHERE EXISTS (
                         SELECT 1 FROM workspaces w
                         JOIN projects p ON p.id = w.project_id AND p.is_deleted = 0
                         WHERE w.id = ?2 AND w.is_deleted = 0 AND w.lifecycle = 'active'
                     )",
                    params![
                        session.id.as_ref(),
                        session.workspace_id.as_ref(),
                        session.agent_ref.as_str(),
                        session.agent_session_id,
                        session.title.as_ref().map(SessionTitle::as_str),
                        session.status.database_value(),
                        session.history_state.database_value(),
                        session.audit_fields.created_at,
                        session.audit_fields.updated_at,
                        bool_to_sqlite(session.audit_fields.is_deleted),
                    ],
                )?;
                if inserted_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    ));
                }

                Ok(session)
            })
            .map_err(session_repository_error_from_database)
    }

    /// Loads one visible session row by identifier.
    fn find_session(&self, session_id: &SessionId) -> Result<Option<Session>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workspace_id, agent_cli, agent_session_id, title, status, history_degraded_reason, created_at, updated_at, is_deleted
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
    fn list_sessions(&self) -> Result<Vec<Session>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workspace_id, agent_cli, agent_session_id, title, status, history_degraded_reason, created_at, updated_at, is_deleted
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

    /// Lists visible sessions that are not bound to a visible workflow node run.
    fn list_standalone_sessions(&self) -> Result<Vec<Session>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workspace_id, agent_cli, agent_session_id, title, status, history_degraded_reason, created_at, updated_at, is_deleted
                     FROM sessions s
                     WHERE s.is_deleted = 0
                       AND NOT EXISTS (
                           SELECT 1
                           FROM workflow_node_runs nr
                           WHERE nr.session_id = s.id AND nr.is_deleted = 0
                       )
                     ORDER BY s.created_at, s.id",
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

    /// Updates only the title so lifecycle or binding changes cannot be overwritten by a stale snapshot.
    fn update_session_title(
        &self,
        session_id: &SessionId,
        title: &SessionTitle,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "UPDATE sessions SET title = ?2, updated_at = ?3
                     WHERE id = ?1 AND is_deleted = 0
                    RETURNING id, workspace_id, agent_cli, agent_session_id, title, status,
                         history_degraded_reason, created_at, updated_at, is_deleted",
                )?;
                let mut rows =
                    statement.query(params![session_id.as_ref(), title.as_str(), now])?;
                match rows.next()? {
                    Some(row) => map_session_row(row),
                    None => Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    )),
                }
            })
            .map_err(session_repository_error_from_database)
    }

    /// Updates only lifecycle status so unrelated session state remains authoritative.
    fn update_session_status(
        &self,
        session_id: &SessionId,
        status: SessionStatus,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                // The whole ownership chain must still be visible: admission racing
                // an aggregate deletion must fail here instead of resurrecting a
                // session row whose task or project rows are already soft-deleted.
                let mut statement = connection.prepare(
                    "UPDATE sessions SET status = ?2, updated_at = ?3
                     WHERE id = ?1 AND is_deleted = 0
                       AND EXISTS (
                           SELECT 1 FROM workspaces w
                           JOIN projects p ON p.id = w.project_id AND p.is_deleted = 0
                           WHERE w.id = sessions.workspace_id
                             AND w.is_deleted = 0 AND w.lifecycle = 'active'
                       )
                     RETURNING id, workspace_id, agent_cli, agent_session_id, title, status,
                         history_degraded_reason, created_at, updated_at, is_deleted",
                )?;
                let mut rows =
                    statement.query(params![session_id.as_ref(), status.database_value(), now])?;
                match rows.next()? {
                    Some(row) => map_session_row(row),
                    None => Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    )),
                }
            })
            .map_err(session_repository_error_from_database)
    }

    /// Updates only the provider binding while preserving title, lifecycle, and history state.
    fn update_session_binding(
        &self,
        session_id: &SessionId,
        agent_ref: AgentRef,
        agent_session_id: &str,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "UPDATE sessions SET agent_cli = ?2, agent_session_id = ?3, updated_at = ?4
                     WHERE id = ?1 AND is_deleted = 0
                    RETURNING id, workspace_id, agent_cli, agent_session_id, title, status,
                         history_degraded_reason, created_at, updated_at, is_deleted",
                )?;
                let mut rows = statement.query(params![
                    session_id.as_ref(),
                    agent_ref.as_str(),
                    agent_session_id,
                    now
                ])?;
                match rows.next()? {
                    Some(row) => map_session_row(row),
                    None => Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    )),
                }
            })
            .map_err(session_repository_error_from_database)
    }

    /// Updates only the history state so a stale actor cannot erase a newer title.
    fn update_session_history_state(
        &self,
        session_id: &SessionId,
        history_state: &HistoryState,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "UPDATE sessions SET history_degraded_reason = ?2, updated_at = ?3
                     WHERE id = ?1 AND is_deleted = 0
                    RETURNING id, workspace_id, agent_cli, agent_session_id, title, status,
                         history_degraded_reason, created_at, updated_at, is_deleted",
                )?;
                let mut rows = statement.query(params![
                    session_id.as_ref(),
                    history_state.database_value(),
                    now
                ])?;
                match rows.next()? {
                    Some(row) => map_session_row(row),
                    None => Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    )),
                }
            })
            .map_err(session_repository_error_from_database)
    }

    /// Soft-deletes one visible session row and reports whether it existed.
    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
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
    let agent_ref = AgentRef::parse(row.get::<_, String>("agent_cli")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    let title = row
        .get::<_, Option<String>>("title")?
        .map(|title| SessionTitle::parse(title).map_err(DomainModelError::from))
        .transpose()?;

    let history_state =
        HistoryState::from_database_value(row.get::<_, Option<String>>("history_degraded_reason")?);

    Ok(Session::new(
        SessionId::new(row.get::<_, String>("id")?),
        WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        agent_ref,
        row.get::<_, String>("agent_session_id")?,
        status,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    )
    .with_title(title)
    .restoring_history_state(history_state))
}

/// Converts shared database-layer failures into session repository errors.
fn session_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

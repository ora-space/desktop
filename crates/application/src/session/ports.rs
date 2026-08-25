use crate::RepositoryError;
use ora_domain::{AgentRef, HistoryState, Session, SessionId, SessionStatus, SessionTitle};

/// Supplies application-owned persistence operations for session CRUD use cases.
///
/// Implementations are expected to hide storage details such as soft-delete columns
/// while preserving the transport-agnostic behavior required by the handlers.
pub trait SessionRepository {
    /// Persists a newly created session and returns the stored snapshot.
    fn create_session(&self, session: Session) -> Result<Session, RepositoryError>;

    /// Loads one visible session by identifier.
    fn find_session(&self, session_id: &SessionId) -> Result<Option<Session>, RepositoryError>;

    /// Lists every visible session in storage order.
    fn list_sessions(&self) -> Result<Vec<Session>, RepositoryError>;

    /// Lists visible sessions that are not owned by workflow node runs.
    ///
    /// Workflow node sessions remain individually addressable for Theater and history replay, but
    /// ordinary conversation surfaces must not project them as user-created chats.
    fn list_standalone_sessions(&self) -> Result<Vec<Session>, RepositoryError>;

    /// Updates only the durable display title and returns the complete current row.
    fn update_session_title(
        &self,
        session_id: &SessionId,
        title: &SessionTitle,
        now: i64,
    ) -> Result<Session, RepositoryError>;

    /// Updates only lifecycle status and returns the complete current row.
    fn update_session_status(
        &self,
        session_id: &SessionId,
        status: SessionStatus,
        now: i64,
    ) -> Result<Session, RepositoryError>;

    /// Updates only provider binding and returns the complete current row.
    fn update_session_binding(
        &self,
        session_id: &SessionId,
        agent_ref: AgentRef,
        agent_session_id: &str,
        now: i64,
    ) -> Result<Session, RepositoryError>;

    /// Updates only history state and returns the complete current row.
    fn update_session_history_state(
        &self,
        session_id: &SessionId,
        history_state: &HistoryState,
        now: i64,
    ) -> Result<Session, RepositoryError>;

    /// Marks a session deleted and returns whether a visible session was affected.
    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError>;
}

/// Supplies new session identifiers for create use cases.
pub trait SessionIdGenerator {
    /// Produces the identifier for a newly created session.
    fn generate_session_id(&self) -> SessionId;
}

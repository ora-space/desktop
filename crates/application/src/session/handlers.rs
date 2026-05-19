use crate::session::mapper::map_session;
use crate::session::ports::{SessionIdGenerator, SessionRepository};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    SessionStatus, UpdateSessionRequest, UpdateSessionResponse,
};
use ora_domain::{
    AgentId, AuditFields, Session as DomainSession, SessionId,
    SessionStatus as DomainSessionStatus, TaskId,
};
use ora_logging::{ora_error, ora_info};

/// Handles session creation without depending on transport-specific concerns.
pub struct CreateSessionHandler<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    CreateSessionHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    CreateSessionHandler<Repository, IdGenerator, ClockSource>
where
    Repository: SessionRepository,
    IdGenerator: SessionIdGenerator,
    ClockSource: Clock,
{
    /// Creates a new session snapshot and returns the public response payload.
    pub fn handle(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let session = DomainSession::new(
            self.id_generator.generate_session_id(),
            TaskId::new(request.task_id),
            AgentId::new(request.agent_id),
            request.agent_session_id,
            map_contract_session_status(request.status),
            AuditFields::new(now, now, false),
        );
        let session = self.repository.create_session(session).map_err(|error| {
            let error = ApplicationError::from_session_repository_error(error);
            log_session_failure("create_session", None, &error);
            error
        })?;

        log_session_success("create_session", Some(&session.id));

        Ok(CreateSessionResponse {
            session: map_session(session),
        })
    }
}

/// Handles one session lookup without depending on transport-specific concerns.
pub struct GetSessionHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetSessionHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetSessionHandler<Repository>
where
    Repository: SessionRepository,
{
    /// Loads one visible session or returns a stable not-found application error.
    pub fn handle(
        &self,
        request: GetSessionRequest,
    ) -> Result<GetSessionResponse, ApplicationError> {
        let session_id = SessionId::new(request.session_id);
        let session = self.repository.find_session(&session_id).map_err(|error| {
            let error = ApplicationError::from_session_repository_error(error);
            log_session_failure("get_session", Some(&session_id), &error);
            error
        })?;

        match session {
            Some(session) => {
                log_session_success("get_session", Some(&session_id));

                Ok(GetSessionResponse {
                    session: map_session(session),
                })
            }
            None => {
                let error = ApplicationError::SessionNotFound {
                    session_id: session_id.to_string(),
                };
                log_session_failure("get_session", Some(&session_id), &error);
                Err(error)
            }
        }
    }
}

/// Handles session listing without depending on transport-specific concerns.
pub struct ListSessionsHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListSessionsHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListSessionsHandler<Repository>
where
    Repository: SessionRepository,
{
    /// Lists every visible session and maps each one into the shared contract view.
    pub fn handle(
        &self,
        _request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, ApplicationError> {
        let sessions = self.repository.list_sessions().map_err(|error| {
            let error = ApplicationError::from_session_repository_error(error);
            log_session_failure("list_sessions", None, &error);
            error
        })?;

        ora_info!(
            message = "listed sessions",
            operation = "list_sessions",
            session_count = sessions.len()
        );

        Ok(ListSessionsResponse {
            sessions: sessions.into_iter().map(map_session).collect(),
        })
    }
}

/// Handles session updates without depending on transport-specific concerns.
pub struct UpdateSessionHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateSessionHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateSessionHandler<Repository, ClockSource>
where
    Repository: SessionRepository,
    ClockSource: Clock,
{
    /// Replaces the public session fields while preserving persistence-managed audit state.
    pub fn handle(
        &self,
        request: UpdateSessionRequest,
    ) -> Result<UpdateSessionResponse, ApplicationError> {
        let session_id = SessionId::new(request.session_id);
        let existing_session = self.repository.find_session(&session_id).map_err(|error| {
            let error = ApplicationError::from_session_repository_error(error);
            log_session_failure("update_session", Some(&session_id), &error);
            error
        })?;

        let existing_session = match existing_session {
            Some(existing_session) => existing_session,
            None => {
                let error = ApplicationError::SessionNotFound {
                    session_id: session_id.to_string(),
                };
                log_session_failure("update_session", Some(&session_id), &error);
                return Err(error);
            }
        };

        let session = DomainSession::new(
            session_id.clone(),
            TaskId::new(request.task_id),
            AgentId::new(request.agent_id),
            request.agent_session_id,
            map_contract_session_status(request.status),
            AuditFields::new(
                existing_session.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                existing_session.audit_fields.is_deleted,
            ),
        );
        let session = self.repository.update_session(session).map_err(|error| {
            let error = ApplicationError::from_session_repository_error(error);
            log_session_failure("update_session", Some(&session_id), &error);
            error
        })?;

        log_session_success("update_session", Some(&session_id));

        Ok(UpdateSessionResponse {
            session: map_session(session),
        })
    }
}

/// Handles session deletion without exposing storage-specific soft-delete semantics.
pub struct DeleteSessionHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteSessionHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteSessionHandler<Repository, ClockSource>
where
    Repository: SessionRepository,
    ClockSource: Clock,
{
    /// Deletes one session through a CRUD-shaped contract while letting storage soft-delete it.
    pub fn handle(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, ApplicationError> {
        let session_id = SessionId::new(request.session_id);
        let deleted = self
            .repository
            .soft_delete_session(&session_id, self.clock.now_timestamp_millis())
            .map_err(|error| {
                let error = ApplicationError::from_session_repository_error(error);
                log_session_failure("delete_session", Some(&session_id), &error);
                error
            })?;

        if deleted {
            log_session_success("delete_session", Some(&session_id));

            Ok(DeleteSessionResponse {
                session_id: session_id.to_string(),
            })
        } else {
            let error = ApplicationError::SessionNotFound {
                session_id: session_id.to_string(),
            };
            log_session_failure("delete_session", Some(&session_id), &error);
            Err(error)
        }
    }
}

/// Emits the shared informational event shape for successful session CRUD operations.
fn log_session_success(operation: &'static str, session_id: Option<&SessionId>) {
    match session_id {
        Some(session_id) => {
            ora_info!(
                message = "session operation completed",
                operation,
                session_id = session_id.to_string()
            );
        }
        None => {
            ora_info!(message = "session operation completed", operation);
        }
    }
}

/// Emits the shared error event shape for failed session CRUD operations.
fn log_session_failure(
    operation: &'static str,
    session_id: Option<&SessionId>,
    error: &ApplicationError,
) {
    match (session_id, error) {
        (Some(session_id), ApplicationError::SessionNotFound { .. }) => {
            ora_error!(
                message = "session operation failed",
                operation,
                session_id = session_id.to_string(),
                error.kind = "session_not_found",
                error.message = error.to_string()
            );
        }
        (Some(session_id), ApplicationError::SessionRepository { .. }) => {
            ora_error!(
                message = "session operation failed",
                operation,
                session_id = session_id.to_string(),
                error.kind = "session_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::SessionRepository { .. }) => {
            ora_error!(
                message = "session operation failed",
                operation,
                error.kind = "session_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::SessionNotFound { .. }) => {
            ora_error!(
                message = "session operation failed",
                operation,
                error.kind = "session_not_found",
                error.message = error.to_string()
            );
        }
        _ => {}
    }
}

/// Translates the transport-facing session status into the domain enum.
fn map_contract_session_status(status: SessionStatus) -> DomainSessionStatus {
    match status {
        SessionStatus::Running => DomainSessionStatus::Running,
        SessionStatus::Stopped => DomainSessionStatus::Stopped,
    }
}

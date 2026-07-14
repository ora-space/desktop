use crate::bootstrap::SystemClock;
use ora_application::{
    ApplicationError, CreateSessionHandler, DeleteSessionHandler, GetSessionHandler,
    ListSessionsHandler, UpdateSessionHandler, UuidSessionIdGenerator,
};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    UpdateSessionRequest, UpdateSessionResponse,
};
use ora_db::{RepositoryPool, SqliteSessionRepository};

/// Groups the transport-facing session entry points for the web adapter.
pub struct SessionApi {
    create_session:
        CreateSessionHandler<SqliteSessionRepository, UuidSessionIdGenerator, SystemClock>,
    get_session: GetSessionHandler<SqliteSessionRepository>,
    list_sessions: ListSessionsHandler<SqliteSessionRepository>,
    update_session: UpdateSessionHandler<SqliteSessionRepository, SystemClock>,
    delete_session: DeleteSessionHandler<SqliteSessionRepository, SystemClock>,
}

impl SessionApi {
    /// Builds the session transport API from the shared repository pool and clock source.
    pub fn new(pool: RepositoryPool, clock: SystemClock) -> Self {
        let repository = SqliteSessionRepository::new(pool);

        Self {
            create_session: CreateSessionHandler::new(
                repository.clone(),
                UuidSessionIdGenerator::new(),
                clock,
            ),
            get_session: GetSessionHandler::new(repository.clone()),
            list_sessions: ListSessionsHandler::new(repository.clone()),
            update_session: UpdateSessionHandler::new(repository.clone(), clock),
            delete_session: DeleteSessionHandler::new(repository, clock),
        }
    }

    /// Accepts a create-session request and delegates the use case to the application layer.
    pub fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, ApplicationError> {
        self.create_session.handle(request)
    }

    /// Accepts a get-session request and delegates the use case to the application layer.
    pub fn get_session(
        &self,
        request: GetSessionRequest,
    ) -> Result<GetSessionResponse, ApplicationError> {
        self.get_session.handle(request)
    }

    /// Accepts a list-sessions request and delegates the use case to the application layer.
    pub fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, ApplicationError> {
        self.list_sessions.handle(request)
    }

    /// Accepts an update-session request and delegates the use case to the application layer.
    pub fn update_session(
        &self,
        request: UpdateSessionRequest,
    ) -> Result<UpdateSessionResponse, ApplicationError> {
        self.update_session.handle(request)
    }

    /// Accepts a delete-session request and delegates the use case to the application layer.
    pub fn delete_session(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, ApplicationError> {
        self.delete_session.handle(request)
    }
}

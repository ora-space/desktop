use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, GetSessionHandler, ListSessionsHandler, RenameSessionHandler,
};
use ora_contracts::{
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    RenameSessionRequest, RenameSessionResponse,
};
use ora_db::{RepositoryPool, SqliteSessionRepository};

/// Groups persisted session query and title-mutation handlers; runtime mutations live in agent_runtime.
pub(crate) struct SessionApi {
    get: GetSessionHandler<SqliteSessionRepository>,
    list: ListSessionsHandler<SqliteSessionRepository>,
    rename: RenameSessionHandler<SqliteSessionRepository, SystemClock>,
}

impl SessionApi {
    /// Builds session handlers from the shared repository pool.
    pub(crate) fn new(pool: RepositoryPool) -> Self {
        let repository = SqliteSessionRepository::new(pool);
        Self {
            get: GetSessionHandler::new(repository.clone()),
            list: ListSessionsHandler::new(repository.clone()),
            rename: RenameSessionHandler::new(repository, SystemClock),
        }
    }

    /// Executes one session lookup through the application handler.
    pub(crate) fn get(
        &self,
        request: GetSessionRequest,
    ) -> Result<GetSessionResponse, ApplicationError> {
        self.get.handle(request)
    }

    /// Executes session listing through the application handler.
    pub(crate) fn list(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, ApplicationError> {
        self.list.handle(request)
    }

    /// Executes one user-driven session title update through the application handler.
    pub(crate) fn rename(
        &self,
        request: RenameSessionRequest,
    ) -> Result<RenameSessionResponse, ApplicationError> {
        self.rename.handle(request)
    }
}

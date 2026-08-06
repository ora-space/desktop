use crate::clock::SystemClock;
use ora_application::{
    AgentImportService, ApplicationError, CreateAgentDefinitionHandler,
    DeleteAgentDefinitionHandler, GetAgentDefinitionHandler, ListAgentDefinitionsHandler,
    UpdateAgentDefinitionHandler, UuidAgentDefinitionIdGenerator,
};
use ora_contracts::{
    CommitAgentImportRequest, CommitAgentImportResponse, CreateAgentRequest, CreateAgentResponse,
    DeleteAgentRequest, DeleteAgentResponse, GetAgentRequest, GetAgentResponse, ListAgentsRequest,
    ListAgentsResponse, PrepareAgentImportRequest, PrepareAgentImportResponse, UpdateAgentRequest,
    UpdateAgentResponse,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository};

/// Groups the concrete configurable-agent handlers shared by runtime adapters.
pub(crate) struct AgentApi {
    create: CreateAgentDefinitionHandler<
        SqliteAgentDefinitionRepository,
        UuidAgentDefinitionIdGenerator,
        SystemClock,
    >,
    get: GetAgentDefinitionHandler<SqliteAgentDefinitionRepository>,
    list: ListAgentDefinitionsHandler<SqliteAgentDefinitionRepository>,
    update: UpdateAgentDefinitionHandler<SqliteAgentDefinitionRepository, SystemClock>,
    delete: DeleteAgentDefinitionHandler<SqliteAgentDefinitionRepository, SystemClock>,
    import: AgentImportService<
        SqliteAgentDefinitionRepository,
        UuidAgentDefinitionIdGenerator,
        SystemClock,
    >,
}

impl AgentApi {
    /// Builds configurable-agent handlers from the shared repository pool.
    pub(crate) fn new(pool: RepositoryPool, clock: SystemClock) -> Self {
        let repository = SqliteAgentDefinitionRepository::new(pool);

        Self {
            create: CreateAgentDefinitionHandler::new(
                repository.clone(),
                UuidAgentDefinitionIdGenerator::new(),
                clock,
            ),
            get: GetAgentDefinitionHandler::new(repository.clone()),
            list: ListAgentDefinitionsHandler::new(repository.clone()),
            update: UpdateAgentDefinitionHandler::new(repository.clone(), clock),
            delete: DeleteAgentDefinitionHandler::new(repository.clone(), clock),
            import: AgentImportService::new(
                repository,
                UuidAgentDefinitionIdGenerator::new(),
                clock,
            ),
        }
    }

    /// Executes configurable-agent creation through the application handler.
    pub(crate) fn create(
        &self,
        request: CreateAgentRequest,
    ) -> Result<CreateAgentResponse, ApplicationError> {
        self.create.handle(request)
    }

    /// Executes one configurable-agent lookup through the application handler.
    pub(crate) fn get(
        &self,
        request: GetAgentRequest,
    ) -> Result<GetAgentResponse, ApplicationError> {
        self.get.handle(request)
    }

    /// Executes configurable-agent listing through the application handler.
    pub(crate) fn list(
        &self,
        request: ListAgentsRequest,
    ) -> Result<ListAgentsResponse, ApplicationError> {
        self.list.handle(request)
    }

    /// Executes configurable-agent replacement through the application handler.
    pub(crate) fn update(
        &self,
        request: UpdateAgentRequest,
    ) -> Result<UpdateAgentResponse, ApplicationError> {
        self.update.handle(request)
    }

    /// Executes configurable-agent deletion through the application handler.
    pub(crate) fn delete(
        &self,
        request: DeleteAgentRequest,
    ) -> Result<DeleteAgentResponse, ApplicationError> {
        self.delete.handle(request)
    }

    pub(crate) fn prepare_import(
        &self,
        request: PrepareAgentImportRequest,
    ) -> Result<PrepareAgentImportResponse, ApplicationError> {
        self.import.prepare(request)
    }

    pub(crate) fn commit_import(
        &self,
        request: CommitAgentImportRequest,
    ) -> Result<CommitAgentImportResponse, ApplicationError> {
        self.import.commit(request)
    }
}

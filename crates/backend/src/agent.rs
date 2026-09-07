use crate::BackendError;
use crate::clock::SystemClock;
use ora_application::{
    AgentImportService, CreateAgentDefinitionHandler, DeleteAgentDefinitionHandler,
    GetAgentDefinitionHandler, ListAgentDefinitionsHandler, UpdateAgentDefinitionHandler,
    UuidAgentDefinitionIdGenerator,
};
use ora_contracts::{
    CommitAgentImportRequest, CommitAgentImportResponse, CreateAgentRequest, CreateAgentResponse,
    DeleteAgentRequest, DeleteAgentResponse, GetAgentRequest, GetAgentResponse, ListAgentsRequest,
    ListAgentsResponse, PrepareAgentImportRequest, PrepareAgentImportResponse, UpdateAgentRequest,
    UpdateAgentResponse,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository};

/// Groups the concrete configurable-agent handlers shared by runtime adapters.
pub struct AgentApi {
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
    pub fn create(&self, request: CreateAgentRequest) -> Result<CreateAgentResponse, BackendError> {
        self.create.handle(request).map_err(BackendError::from)
    }

    /// Executes one configurable-agent lookup through the application handler.
    pub fn get(&self, request: GetAgentRequest) -> Result<GetAgentResponse, BackendError> {
        self.get.handle(request).map_err(BackendError::from)
    }

    /// Executes configurable-agent listing through the application handler.
    pub fn list(&self, request: ListAgentsRequest) -> Result<ListAgentsResponse, BackendError> {
        self.list.handle(request).map_err(BackendError::from)
    }

    /// Executes configurable-agent replacement through the application handler.
    pub fn update(&self, request: UpdateAgentRequest) -> Result<UpdateAgentResponse, BackendError> {
        self.update.handle(request).map_err(BackendError::from)
    }

    /// Executes configurable-agent deletion through the application handler.
    pub fn delete(&self, request: DeleteAgentRequest) -> Result<DeleteAgentResponse, BackendError> {
        self.delete.handle(request).map_err(BackendError::from)
    }

    /// Validates imported Markdown and reports the decisions required before committing it.
    pub fn prepare_import(
        &self,
        request: PrepareAgentImportRequest,
    ) -> Result<PrepareAgentImportResponse, BackendError> {
        self.import.prepare(request).map_err(BackendError::from)
    }

    /// Commits an import through the same identity and conflict checks as the agent catalog.
    pub fn commit_import(
        &self,
        request: CommitAgentImportRequest,
    ) -> Result<CommitAgentImportResponse, BackendError> {
        self.import.commit(request).map_err(BackendError::from)
    }
}

#[cfg(test)]
mod tests;

use super::prerequisites::SkillRoleWorkspaceInitializer;
use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, CreateWorkflowRunHandler, DeleteWorkflowRunHandler, GetWorkflowRunHandler,
    ListWorkflowNodeRunsHandler, ListWorkflowRunsByWorkflowHandler, ListWorkflowRunsHandler,
    RenameWorkflowRunHandler, UuidWorkflowRunIdGenerator,
};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
    RenameWorkflowRunRequest, RenameWorkflowRunResponse,
};
use ora_db::{
    RepositoryPool, SqliteWorkflowRepository, SqliteWorkflowRunRepository,
    SqliteWorkspaceRepository,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Groups workflow-run handlers while resolving the selected workspace.
pub(crate) struct WorkflowRunApi {
    pool: RepositoryPool,
    /// Skill catalog root used to validate and resolve workflow skill bindings.
    skills_root: PathBuf,
    get: GetWorkflowRunHandler<SqliteWorkflowRunRepository>,
    list: ListWorkflowRunsHandler<SqliteWorkflowRunRepository>,
    list_by_workflow: ListWorkflowRunsByWorkflowHandler<SqliteWorkflowRunRepository>,
    list_node_runs: ListWorkflowNodeRunsHandler<SqliteWorkflowRunRepository>,
    clock: SystemClock,
}

impl WorkflowRunApi {
    /// Builds run handlers from shared persistence and the skill catalog root used for validation.
    pub(crate) fn new(pool: RepositoryPool, skills_root: PathBuf, clock: SystemClock) -> Self {
        let repository = Arc::new(SqliteWorkflowRunRepository::new(pool.clone()));

        Self {
            pool,
            skills_root,
            get: GetWorkflowRunHandler::new(repository.clone()),
            list: ListWorkflowRunsHandler::new(repository.clone()),
            list_by_workflow: ListWorkflowRunsByWorkflowHandler::new(repository.clone()),
            list_node_runs: ListWorkflowNodeRunsHandler::new(repository),
            clock,
        }
    }

    /// Validates the selected workspace and its workflow prerequisites, then persists the run.
    pub(crate) fn create(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, ApplicationError> {
        let initializer =
            SkillRoleWorkspaceInitializer::new(self.skills_root.clone(), self.pool.clone())
                .map_err(|error| ApplicationError::WorkflowRunStartFailed {
                    message: error.to_string(),
                })?;
        let handler = CreateWorkflowRunHandler::new(
            Arc::new(SqliteWorkflowRepository::new(self.pool.clone())),
            Arc::new(SqliteWorkspaceRepository::new(self.pool.clone())),
            Arc::new(SqliteWorkflowRunRepository::new(self.pool.clone())),
            UuidWorkflowRunIdGenerator::new(),
            initializer,
            self.clock,
        );

        handler.handle(request)
    }

    /// Loads one run detail through the shared application composition.
    pub(crate) fn get(
        &self,
        request: GetWorkflowRunRequest,
    ) -> Result<GetWorkflowRunResponse, ApplicationError> {
        self.get.handle(request)
    }

    /// Lists run summaries for the requested project.
    pub(crate) fn list(
        &self,
        request: ListWorkflowRunsRequest,
    ) -> Result<ListWorkflowRunsResponse, ApplicationError> {
        self.list.handle(request)
    }

    /// Lists run summaries for the requested workflow.
    pub(crate) fn list_by_workflow(
        &self,
        request: ListWorkflowRunsByWorkflowRequest,
    ) -> Result<ListWorkflowRunsByWorkflowResponse, ApplicationError> {
        self.list_by_workflow.handle(request)
    }

    /// Lists the node-run history of one run.
    pub(crate) fn list_node_runs(
        &self,
        request: ListWorkflowNodeRunsRequest,
    ) -> Result<ListWorkflowNodeRunsResponse, ApplicationError> {
        self.list_node_runs.handle(request)
    }

    /// Soft-deletes one run; the cascade registers durable Git cleanup jobs.
    pub(crate) fn delete(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, ApplicationError> {
        let handler = DeleteWorkflowRunHandler::new(
            Arc::new(SqliteWorkflowRunRepository::new(self.pool.clone())),
            self.clock,
        );

        handler.handle(request)
    }

    /// Renames one workspace-owned workflow run through the application boundary.
    pub(crate) fn rename(
        &self,
        request: RenameWorkflowRunRequest,
    ) -> Result<RenameWorkflowRunResponse, ApplicationError> {
        let handler = RenameWorkflowRunHandler::new(
            Arc::new(SqliteWorkflowRunRepository::new(self.pool.clone())),
            self.clock,
        );

        handler.handle(request)
    }
}

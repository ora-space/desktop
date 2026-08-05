use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, CreateWorkflowRunHandler, DeleteWorkflowRunHandler, GetWorkflowRunHandler,
    GitTaskWorktreeProvisioner, ListWorkflowNodeRunsHandler, ListWorkflowRunsHandler,
    ProjectRepository, RepositoryError, TaskRepository, UuidTaskIdGenerator,
    UuidWorkflowRunIdGenerator, UuidWorktreeIdGenerator, WorkflowRunRepository,
};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsRequest,
    ListWorkflowRunsResponse,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteTaskRepository, SqliteWorkflowRepository,
    SqliteWorkflowRunRepository,
};
use ora_domain::{Project, ProjectId, TaskId, WorkflowRunId};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Groups workflow-run handlers while resolving the owning project's Git repository for worktrees.
pub(crate) struct WorkflowRunApi {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
    get: GetWorkflowRunHandler<SqliteWorkflowRunRepository>,
    list: ListWorkflowRunsHandler<SqliteWorkflowRunRepository>,
    list_node_runs: ListWorkflowNodeRunsHandler<SqliteWorkflowRunRepository>,
    clock: SystemClock,
}

impl WorkflowRunApi {
    /// Builds run handlers from shared persistence and the mutable worktree-root configuration.
    pub(crate) fn new(
        pool: RepositoryPool,
        worktree_root: Arc<RwLock<PathBuf>>,
        clock: SystemClock,
    ) -> Self {
        let repository = Arc::new(SqliteWorkflowRunRepository::new(pool.clone()));

        Self {
            pool,
            worktree_root,
            get: GetWorkflowRunHandler::new(repository.clone()),
            list: ListWorkflowRunsHandler::new(repository.clone()),
            list_node_runs: ListWorkflowNodeRunsHandler::new(repository),
            clock,
        }
    }

    /// Resolves the run's project repository and provisions a dedicated worktree before persisting.
    pub(crate) fn create(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, ApplicationError> {
        let project = self.find_project(&ProjectId::new(&request.project_id))?;
        let handler = CreateWorkflowRunHandler::new(
            Arc::new(SqliteWorkflowRepository::new(self.pool.clone())),
            Arc::new(SqliteWorkflowRunRepository::new(self.pool.clone())),
            UuidWorkflowRunIdGenerator::new(),
            UuidTaskIdGenerator::new(),
            UuidWorktreeIdGenerator::new(),
            GitTaskWorktreeProvisioner::new(PathBuf::from(project.root_path)),
            self.worktree_root_snapshot()?,
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

    /// Lists the node-run history of one run.
    pub(crate) fn list_node_runs(
        &self,
        request: ListWorkflowNodeRunsRequest,
    ) -> Result<ListWorkflowNodeRunsResponse, ApplicationError> {
        self.list_node_runs.handle(request)
    }

    /// Soft-deletes one run and removes its physical worktree from the owning project repository.
    pub(crate) fn delete(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(&request.run_id);
        let run_repository = SqliteWorkflowRunRepository::new(self.pool.clone());
        let task_id = run_repository
            .find_run_task_id(&run_id)
            .map_err(|source| ApplicationError::WorkflowRunRepository { source })?
            .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
                run_id: request.run_id.clone(),
            })?;
        let project = self.find_project_for_task(&task_id, &request.run_id)?;
        let handler = DeleteWorkflowRunHandler::new(
            Arc::new(run_repository),
            GitTaskWorktreeProvisioner::new(PathBuf::from(project.root_path)),
            self.clock,
        );

        handler.handle(request)
    }

    /// Loads a visible project or returns the same stable not-found error as project handlers.
    fn find_project(&self, project_id: &ProjectId) -> Result<Project, ApplicationError> {
        let repository = SqliteProjectRepository::new(self.pool.clone());
        let project = repository
            .find_project(project_id)
            .map_err(project_repository_error)?;

        project.ok_or_else(|| ApplicationError::ProjectNotFound {
            project_id: project_id.to_string(),
        })
    }

    /// Resolves the project that owns a run-task so its worktree can be removed from the right repo.
    fn find_project_for_task(
        &self,
        task_id: &TaskId,
        run_id: &str,
    ) -> Result<Project, ApplicationError> {
        let task = SqliteTaskRepository::new(self.pool.clone())
            .find_task(task_id)
            .map_err(|source| ApplicationError::TaskRepository { source })?
            .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })?;

        self.find_project(&task.project_id)
    }

    /// Captures the configured creation root once so an in-flight operation remains coherent.
    fn worktree_root_snapshot(&self) -> Result<PathBuf, ApplicationError> {
        self.worktree_root
            .read()
            .map(|root| root.clone())
            .map_err(|_poisoned| ApplicationError::TaskWorktreeRootUnavailable)
    }
}

/// Converts project repository failures encountered during dynamic run routing.
fn project_repository_error(error: RepositoryError) -> ApplicationError {
    ApplicationError::ProjectRepository { source: error }
}

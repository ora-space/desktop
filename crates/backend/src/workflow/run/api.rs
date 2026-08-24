use super::prerequisites::SkillRoleWorktreeInitializer;
use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, CreateWorkflowRunHandler, DeleteWorkflowRunHandler, GetWorkflowRunHandler,
    GitTaskWorktreeProvisioner, ListWorkflowNodeRunsHandler, ListWorkflowRunsByWorkflowHandler,
    ListWorkflowRunsHandler, ProjectRepository, RepositoryError, UuidTaskIdGenerator,
    UuidWorkflowRunIdGenerator, UuidWorktreeIdGenerator,
};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteWorkflowRepository, SqliteWorkflowRunRepository,
    SqliteWorktreeProvisioningLeaseRepository,
};
use ora_domain::{Project, ProjectId};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Groups workflow-run handlers while resolving the owning project's Git repository for worktrees.
pub(crate) struct WorkflowRunApi {
    pool: RepositoryPool,
    worktree_root: Arc<RwLock<PathBuf>>,
    /// Skill catalog root used to materialize a run worktree's initial `.agents/skills/`.
    skills_root: PathBuf,
    /// Serializes Git mutations per repository between provisioning and cleanup.
    repository_gates: Arc<crate::git_cleanup::KeyedResourceLocks>,
    get: GetWorkflowRunHandler<SqliteWorkflowRunRepository>,
    list: ListWorkflowRunsHandler<SqliteWorkflowRunRepository>,
    list_by_workflow: ListWorkflowRunsByWorkflowHandler<SqliteWorkflowRunRepository>,
    list_node_runs: ListWorkflowNodeRunsHandler<SqliteWorkflowRunRepository>,
    clock: SystemClock,
}

impl WorkflowRunApi {
    /// Builds run handlers from shared persistence, the mutable worktree-root configuration, and
    /// the skill catalog root used to set up each run worktree's initial state.
    pub(crate) fn new(
        pool: RepositoryPool,
        worktree_root: Arc<RwLock<PathBuf>>,
        skills_root: PathBuf,
        repository_gates: Arc<crate::git_cleanup::KeyedResourceLocks>,
        clock: SystemClock,
    ) -> Self {
        let repository = Arc::new(SqliteWorkflowRunRepository::new(pool.clone()));

        Self {
            pool,
            worktree_root,
            skills_root,
            repository_gates,
            get: GetWorkflowRunHandler::new(repository.clone()),
            list: ListWorkflowRunsHandler::new(repository.clone()),
            list_by_workflow: ListWorkflowRunsByWorkflowHandler::new(repository.clone()),
            list_node_runs: ListWorkflowNodeRunsHandler::new(repository),
            clock,
        }
    }

    /// Resolves the run's project repository, provisions a dedicated worktree, and sets up its
    /// initial `.agents/skills/` state before persisting.
    pub(crate) fn create(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, ApplicationError> {
        let project = self.find_project(&ProjectId::new(&request.project_id))?;
        let repository_root = PathBuf::from(project.root_path);
        let initializer =
            SkillRoleWorktreeInitializer::new(self.skills_root.clone(), self.pool.clone())
                .map_err(|error| ApplicationError::WorkflowRunStartFailed {
                    message: error.to_string(),
                })?;
        let handler = CreateWorkflowRunHandler::new(
            Arc::new(SqliteWorkflowRepository::new(self.pool.clone())),
            Arc::new(SqliteWorkflowRunRepository::new(self.pool.clone())),
            UuidWorkflowRunIdGenerator::new(),
            UuidTaskIdGenerator::new(),
            UuidWorktreeIdGenerator::new(),
            crate::git_cleanup::GatedWorktreeProvisioner::new(
                GitTaskWorktreeProvisioner::new(repository_root.clone()),
                Arc::clone(&self.repository_gates),
                crate::git_cleanup::normalize_repository_key(&repository_root),
            ),
            initializer,
            SqliteWorktreeProvisioningLeaseRepository::new(self.pool.clone()),
            repository_root,
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

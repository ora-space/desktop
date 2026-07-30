use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, BranchLister, BranchListingError, BranchReference, Clock,
    CreateProjectHandler, GetProjectHandler, ListProjectBranchesHandler, ListProjectsHandler,
    UpdateProjectHandler, UuidProjectIdGenerator,
};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectBranchesRequest, ListProjectBranchesResponse,
    ListProjectsRequest, ListProjectsResponse, UpdateProjectRequest, UpdateProjectResponse,
};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteTaskRepository, SqliteWorktreeRepository,
};
use ora_domain::ProjectId;
use std::path::Path;

use crate::{BackendError, BackendErrorKind};
use gitlancer::git::base_branch::{ListWorktreeBasesRequest, ListWorktreeBasesResponse};
use gitlancer::{CliGitRunner, Git, RepoRoot};

type ProjectBranchListHandler = ListProjectBranchesHandler<
    SqliteProjectRepository,
    SqliteTaskRepository,
    SqliteWorktreeRepository,
    GitBranchLister,
>;

/// Groups the concrete project handlers shared by runtime adapters.
pub(crate) struct ProjectApi {
    pool: RepositoryPool,
    create: CreateProjectHandler<SqliteProjectRepository, UuidProjectIdGenerator, SystemClock>,
    get: GetProjectHandler<SqliteProjectRepository>,
    list: ListProjectsHandler<SqliteProjectRepository>,
    list_branches: ProjectBranchListHandler,
    update: UpdateProjectHandler<SqliteProjectRepository, SystemClock>,
    clock: SystemClock,
}

impl ProjectApi {
    /// Builds project handlers from the shared repository pool.
    pub(crate) fn new(pool: RepositoryPool, clock: SystemClock) -> Self {
        let repository = SqliteProjectRepository::new(pool.clone());

        Self {
            pool: pool.clone(),
            create: CreateProjectHandler::new(
                repository.clone(),
                UuidProjectIdGenerator::new(),
                clock,
            ),
            get: GetProjectHandler::new(repository.clone()),
            list: ListProjectsHandler::new(repository.clone()),
            list_branches: ListProjectBranchesHandler::new(
                repository.clone(),
                SqliteTaskRepository::new(pool.clone()),
                SqliteWorktreeRepository::new(pool),
                GitBranchLister,
            ),
            update: UpdateProjectHandler::new(repository, clock),
            clock,
        }
    }

    /// Executes project creation through the application handler.
    pub(crate) fn create(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, ApplicationError> {
        self.create.handle(request)
    }

    /// Executes one project lookup through the application handler.
    pub(crate) fn get(
        &self,
        request: GetProjectRequest,
    ) -> Result<GetProjectResponse, ApplicationError> {
        self.get.handle(request)
    }

    /// Executes project listing through the application handler.
    pub(crate) fn list(
        &self,
        request: ListProjectsRequest,
    ) -> Result<ListProjectsResponse, ApplicationError> {
        self.list.handle(request)
    }

    /// Executes project branch listing through the application handler.
    pub(crate) fn list_branches(
        &self,
        request: ListProjectBranchesRequest,
    ) -> Result<ListProjectBranchesResponse, ApplicationError> {
        self.list_branches.handle(request)
    }

    /// Executes project replacement through the application handler.
    pub(crate) fn update(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, ApplicationError> {
        self.update.handle(request)
    }

    /// Executes project deletion through the application handler.
    pub(crate) fn delete(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, BackendError> {
        let project_id = ProjectId::new(request.project_id);
        let outcome = SqliteCascadeRepository::new(self.pool.clone())
            .delete_project(&project_id, self.clock.now_timestamp_millis())
            .map_err(|_| {
                BackendError::new(
                    BackendErrorKind::Internal,
                    "project_repository_error",
                    "project repository operation failed",
                )
            })?;

        match outcome {
            CascadeDeleteOutcome::Deleted => Ok(DeleteProjectResponse {
                project_id: project_id.to_string(),
            }),
            CascadeDeleteOutcome::NotFound => Err(BackendError::new(
                BackendErrorKind::NotFound,
                "project_not_found",
                format!("project not found: {project_id}"),
            )),
            CascadeDeleteOutcome::ActiveSession => Err(BackendError::new(
                BackendErrorKind::Conflict,
                "resource_in_use",
                "project has a running session and cannot be deleted",
            )),
        }
    }
}

/// Adapts Gitlancer's refreshed worktree bases to the application branch-listing port.
#[derive(Debug, Clone, Copy)]
struct GitBranchLister;

impl BranchLister for GitBranchLister {
    /// Discovers the repository and returns logical names paired with resolvable refs.
    fn list_branches(
        &self,
        repository_root: &Path,
    ) -> Result<Vec<BranchReference>, BranchListingError> {
        let git = Git::new(CliGitRunner);
        let repository = git
            .discover_repository(RepoRoot::new(repository_root))
            .map_err(|_| BranchListingError::NotARepository)?;
        let ListWorktreeBasesResponse { bases } = git
            .list_worktree_bases(ListWorktreeBasesRequest {
                repository: &repository,
            })
            .map_err(|error| BranchListingError::OperationFailed(error.to_string()))?;

        Ok(bases
            .into_iter()
            .map(|base| BranchReference {
                name: base.branch_name().as_str().to_string(),
                ref_name: base.reference_name(),
            })
            .collect())
    }
}

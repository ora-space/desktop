use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, Clock, CreateProjectHandler, GetProjectHandler, ListProjectsHandler,
    TaskRepository, UpdateProjectHandler, UuidProjectIdGenerator, WorktreeRepository,
};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectBranchesRequest, ListProjectBranchesResponse,
    ListProjectsRequest, ListProjectsResponse, ProjectBranch, UpdateProjectRequest,
    UpdateProjectResponse,
};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteTaskRepository, SqliteWorktreeRepository,
};
use ora_domain::ProjectId;
use std::collections::HashMap;

use crate::{BackendError, BackendErrorKind};
use gitlancer::git::branch::{ListBranchesRequest, ListBranchesResponse};
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository};

/// Groups the concrete project handlers shared by runtime adapters.
pub(crate) struct ProjectApi {
    pool: RepositoryPool,
    create: CreateProjectHandler<SqliteProjectRepository, UuidProjectIdGenerator, SystemClock>,
    get: GetProjectHandler<SqliteProjectRepository>,
    list: ListProjectsHandler<SqliteProjectRepository>,
    update: UpdateProjectHandler<SqliteProjectRepository, SystemClock>,
    clock: SystemClock,
}

impl ProjectApi {
    /// Builds project handlers from the shared repository pool.
    pub(crate) fn new(pool: RepositoryPool, clock: SystemClock) -> Self {
        let repository = SqliteProjectRepository::new(pool.clone());

        Self {
            pool,
            create: CreateProjectHandler::new(
                repository.clone(),
                UuidProjectIdGenerator::new(),
                clock,
            ),
            get: GetProjectHandler::new(repository.clone()),
            list: ListProjectsHandler::new(repository.clone()),
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

    /// Lists repository-local branches that can seed a new task worktree.
    pub(crate) fn list_branches(
        &self,
        request: ListProjectBranchesRequest,
    ) -> Result<ListProjectBranchesResponse, BackendError> {
        let project = self
            .get
            .handle(GetProjectRequest {
                project_id: request.project_id,
            })
            .map_err(BackendError::from)?
            .project;
        let project_id = ProjectId::new(&project.id);
        let repository = Repository::new(RepoRoot::new(project.root_path));
        let git = Git::new(CliGitRunner);
        git.discover_repository(repository.root().clone())
            .map_err(|_| {
                BackendError::new(
                    BackendErrorKind::BadRequest,
                    "worktree_requires_git_repository",
                    "worktree mode requires a Git repository",
                )
            })?;
        let ListBranchesResponse { branches } = git
            .list_branches(ListBranchesRequest {
                repository: &repository,
            })
            .map_err(|_| {
                BackendError::new(
                    BackendErrorKind::Internal,
                    "project_branches_error",
                    "failed to list project branches",
                )
            })?;
        let task_titles = SqliteTaskRepository::new(self.pool.clone())
            .list_tasks()
            .map_err(|_| {
                BackendError::new(
                    BackendErrorKind::Internal,
                    "task_repository_error",
                    "task repository operation failed",
                )
            })?
            .into_iter()
            .filter(|task| task.project_id == project_id)
            .map(|task| (task.id, task.title))
            .collect::<HashMap<_, _>>();
        let managed_branch_titles = SqliteWorktreeRepository::new(self.pool.clone())
            .list_worktrees()
            .map_err(|_| {
                BackendError::new(
                    BackendErrorKind::Internal,
                    "worktree_repository_error",
                    "worktree repository operation failed",
                )
            })?
            .into_iter()
            .filter_map(|worktree| {
                Some((
                    worktree.branch_name?,
                    task_titles.get(&worktree.task_id)?.clone(),
                ))
            })
            .collect::<HashMap<_, _>>();

        Ok(ListProjectBranchesResponse {
            branches: branches
                .into_iter()
                .map(|branch| {
                    let name = branch.as_str().to_string();
                    let display_name = managed_branch_titles
                        .get(&name)
                        .cloned()
                        .unwrap_or_else(|| name.clone());
                    ProjectBranch { name, display_name }
                })
                .collect(),
        })
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

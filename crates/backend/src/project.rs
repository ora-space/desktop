use crate::clock::SystemClock;
use ora_application::{
    ApplicationError, Clock, CreateProjectHandler, GetProjectHandler, ListProjectsHandler,
    UpdateProjectHandler, UuidProjectIdGenerator,
};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectsRequest, ListProjectsResponse,
    UpdateProjectRequest, UpdateProjectResponse,
};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
};
use ora_domain::ProjectId;

use crate::git_cleanup::CommittedAggregateDeletion;
use crate::{BackendError, BackendErrorKind};

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

    /// Executes project replacement through the application handler.
    pub(crate) fn update(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, ApplicationError> {
        self.update.handle(request)
    }

    /// Commits the project aggregate deletion and returns its external cleanup work.
    pub(crate) fn commit_delete(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<CommittedAggregateDeletion<DeleteProjectResponse>, BackendError> {
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
            CascadeDeleteOutcome::Deleted {
                git_cleanup_targets,
            } => Ok(CommittedAggregateDeletion {
                response: DeleteProjectResponse {
                    project_id: project_id.to_string(),
                },
                git_cleanup_targets,
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

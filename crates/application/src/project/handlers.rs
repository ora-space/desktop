use crate::ApplicationError;
use crate::project::mapper::map_project;
use crate::project::ports::{Clock, ProjectIdGenerator, ProjectRepository};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, GetProjectRequest, GetProjectResponse,
    ListProjectsRequest, ListProjectsResponse, UpdateProjectRequest, UpdateProjectResponse,
};
use ora_domain::{AuditFields, Project as DomainProject, ProjectId, WorkspaceLocation};

/// Handles project creation without depending on transport-specific concerns.
pub struct CreateProjectHandler<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    CreateProjectHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    CreateProjectHandler<Repository, IdGenerator, ClockSource>
where
    Repository: ProjectRepository,
    IdGenerator: ProjectIdGenerator,
    ClockSource: Clock,
{
    /// Creates a new project snapshot and returns the public response payload.
    pub fn handle(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let project = DomainProject::new(
            self.id_generator.generate_project_id(),
            request.name,
            AuditFields::new(now, now, false),
        );
        let project = self
            .repository
            .create_project(
                project,
                WorkspaceLocation::local_filesystem(request.main_workspace_path),
            )
            .map_err(ApplicationError::from_project_repository_error)?;

        Ok(CreateProjectResponse {
            project: map_project(project),
        })
    }
}

/// Handles one project lookup without depending on transport-specific concerns.
pub struct GetProjectHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetProjectHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetProjectHandler<Repository>
where
    Repository: ProjectRepository,
{
    /// Loads one visible project or returns a stable not-found application error.
    pub fn handle(
        &self,
        request: GetProjectRequest,
    ) -> Result<GetProjectResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let project = self
            .repository
            .find_project(&project_id)
            .map_err(ApplicationError::from_project_repository_error)?;

        match project {
            Some(project) => Ok(GetProjectResponse {
                project: map_project(project),
            }),
            None => {
                let error = ApplicationError::ProjectNotFound {
                    project_id: project_id.to_string(),
                };
                Err(error)
            }
        }
    }
}

/// Handles project listing without depending on transport-specific concerns.
pub struct ListProjectsHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListProjectsHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListProjectsHandler<Repository>
where
    Repository: ProjectRepository,
{
    /// Lists every visible project and maps each one into the shared contract view.
    pub fn handle(
        &self,
        _request: ListProjectsRequest,
    ) -> Result<ListProjectsResponse, ApplicationError> {
        let projects = self
            .repository
            .list_projects()
            .map_err(ApplicationError::from_project_repository_error)?;

        Ok(ListProjectsResponse {
            projects: projects.into_iter().map(map_project).collect(),
        })
    }
}

/// Handles project updates without depending on transport-specific concerns.
pub struct UpdateProjectHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateProjectHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateProjectHandler<Repository, ClockSource>
where
    Repository: ProjectRepository,
    ClockSource: Clock,
{
    /// Renames a project while preserving its main workspace location and audit identity.
    pub fn handle(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let existing_project = self
            .repository
            .find_project(&project_id)
            .map_err(ApplicationError::from_project_repository_error)?;

        let existing_project = match existing_project {
            Some(existing_project) => existing_project,
            None => {
                let error = ApplicationError::ProjectNotFound {
                    project_id: project_id.to_string(),
                };
                return Err(error);
            }
        };

        let project = DomainProject::new(
            project_id,
            request.name,
            AuditFields::new(
                existing_project.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                existing_project.audit_fields.is_deleted,
            ),
        );
        let project = self
            .repository
            .update_project(project)
            .map_err(ApplicationError::from_project_repository_error)?;

        Ok(UpdateProjectResponse {
            project: map_project(project),
        })
    }
}

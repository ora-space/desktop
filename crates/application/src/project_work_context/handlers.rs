use crate::project::{Clock, ProjectRepository};
use crate::project_work_context::mapper::{
    map_project_work_context, map_project_work_context_surface_to_domain,
};
use crate::project_work_context::ports::{
    ProjectWorkContextIdGenerator, ProjectWorkContextRepository,
};
use crate::{ApplicationError, RepositoryError};
use ora_contracts::{
    OpenProjectWorkContextRequest, OpenProjectWorkContextResponse, RenewProjectWorkContextRequest,
    RenewProjectWorkContextResponse,
};
use ora_domain::{ProjectId, ProjectWorkContext, ProjectWorkContextSurface};

const PROJECT_WORK_CONTEXT_LEASE_DURATION_MILLIS: i64 = 120_000;

/// Handles project open and switch operations without depending on transport-specific concerns.
pub struct OpenProjectWorkContextHandler<ProjectRepo, ContextRepo, IdGenerator, ClockSource> {
    project_repository: ProjectRepo,
    context_repository: ContextRepo,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<ProjectRepo, ContextRepo, IdGenerator, ClockSource>
    OpenProjectWorkContextHandler<ProjectRepo, ContextRepo, IdGenerator, ClockSource>
{
    /// Builds the open/switch handler from the repositories, id generator, and clock source.
    pub fn new(
        project_repository: ProjectRepo,
        context_repository: ContextRepo,
        id_generator: IdGenerator,
        clock: ClockSource,
    ) -> Self {
        Self {
            project_repository,
            context_repository,
            id_generator,
            clock,
        }
    }
}

impl<ProjectRepo, ContextRepo, IdGenerator, ClockSource>
    OpenProjectWorkContextHandler<ProjectRepo, ContextRepo, IdGenerator, ClockSource>
where
    ProjectRepo: ProjectRepository,
    ContextRepo: ProjectWorkContextRepository,
    IdGenerator: ProjectWorkContextIdGenerator,
    ClockSource: Clock,
{
    /// Opens or switches one client window into the requested project and refreshes its lease.
    pub fn handle(
        &self,
        request: OpenProjectWorkContextRequest,
    ) -> Result<OpenProjectWorkContextResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let requested_surface = map_project_work_context_surface_to_domain(request.surface);
        let requested_project_id = ProjectId::new(request.project_id);
        let requested_window_id = request.window_id;

        let project = self
            .project_repository
            .find_project(&requested_project_id)
            .map_err(ApplicationError::from_project_repository_error)?;

        if project.is_none() {
            let error = ApplicationError::ProjectNotFound {
                project_id: requested_project_id.to_string(),
            };
            return Err(error);
        }

        let existing_context = self
            .context_repository
            .find_project_work_context(requested_surface, &requested_window_id)
            .map_err(ApplicationError::from_project_work_context_repository_error)?;
        let existing_context_exists = existing_context.is_some();
        let conflicting_context = self
            .context_repository
            .find_active_project_work_context_for_project(&requested_project_id, now)
            .map_err(ApplicationError::from_project_work_context_repository_error)?;

        if let Some(conflicting_context) = conflicting_context
            && requested_surface == ProjectWorkContextSurface::Tauri
            && conflicting_context.surface == ProjectWorkContextSurface::Tauri
            && !is_same_window_context(existing_context.as_ref(), &conflicting_context)
        {
            let error = ApplicationError::ProjectOccupied {
                project_id: requested_project_id.to_string(),
            };
            return Err(error);
        }

        let context = match existing_context {
            Some(existing_context) => ProjectWorkContext::new(
                existing_context.id,
                requested_surface,
                requested_window_id,
                requested_project_id,
                now + PROJECT_WORK_CONTEXT_LEASE_DURATION_MILLIS,
                existing_context.created_at,
                now,
            ),
            None => ProjectWorkContext::new(
                self.id_generator.generate_project_work_context_id(),
                requested_surface,
                requested_window_id,
                requested_project_id,
                now + PROJECT_WORK_CONTEXT_LEASE_DURATION_MILLIS,
                now,
                now,
            ),
        };
        let context = persist_project_work_context(
            &self.context_repository,
            existing_context_exists,
            context,
        )
        .map_err(ApplicationError::from_project_work_context_repository_error)?;

        Ok(OpenProjectWorkContextResponse {
            context: map_project_work_context(context),
        })
    }
}

/// Handles periodic lease renewal without depending on transport-specific concerns.
pub struct RenewProjectWorkContextHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> RenewProjectWorkContextHandler<Repository, ClockSource> {
    /// Builds the renew handler from the context repository and clock source.
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> RenewProjectWorkContextHandler<Repository, ClockSource>
where
    Repository: ProjectWorkContextRepository,
    ClockSource: Clock,
{
    /// Refreshes the lease for one existing client window context using backend time.
    pub fn handle(
        &self,
        request: RenewProjectWorkContextRequest,
    ) -> Result<RenewProjectWorkContextResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let requested_surface = map_project_work_context_surface_to_domain(request.surface);
        let existing_context = self
            .repository
            .find_project_work_context(requested_surface, &request.window_id)
            .map_err(ApplicationError::from_project_work_context_repository_error)?;

        let existing_context = match existing_context {
            Some(existing_context) => existing_context,
            None => {
                let error = ApplicationError::ProjectWorkContextNotFound {
                    surface: requested_surface.database_value().to_string(),
                    window_id: request.window_id,
                };
                return Err(error);
            }
        };

        let context = self
            .repository
            .update_project_work_context(ProjectWorkContext::new(
                existing_context.id,
                existing_context.surface,
                existing_context.window_id,
                existing_context.project_id,
                now + PROJECT_WORK_CONTEXT_LEASE_DURATION_MILLIS,
                existing_context.created_at,
                now,
            ))
            .map_err(ApplicationError::from_project_work_context_repository_error)?;

        Ok(RenewProjectWorkContextResponse {
            context: map_project_work_context(context),
        })
    }
}

/// Returns whether the conflicting active context already belongs to the same client window.
fn is_same_window_context(
    existing_context: Option<&ProjectWorkContext>,
    conflicting_context: &ProjectWorkContext,
) -> bool {
    match existing_context {
        Some(existing_context) => existing_context.id == conflicting_context.id,
        None => false,
    }
}

/// Persists one work context through create or update depending on whether the row already exists.
fn persist_project_work_context<Repository>(
    repository: &Repository,
    is_update: bool,
    context: ProjectWorkContext,
) -> Result<ProjectWorkContext, RepositoryError>
where
    Repository: ProjectWorkContextRepository,
{
    if is_update {
        repository.update_project_work_context(context)
    } else {
        repository.create_project_work_context(context)
    }
}

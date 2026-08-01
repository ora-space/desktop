use crate::RepositoryError;
use ora_domain::{ProjectId, ProjectWorkContext, ProjectWorkContextId, ProjectWorkContextSurface};

/// Supplies application-owned persistence operations for project work context flows.
///
/// Implementations are expected to hide storage details while preserving the lease semantics
/// required by open, switch, renew, and cleanup operations.
pub trait ProjectWorkContextRepository {
    /// Persists a newly created project work context and returns the stored snapshot.
    fn create_project_work_context(
        &self,
        context: ProjectWorkContext,
    ) -> Result<ProjectWorkContext, RepositoryError>;

    /// Loads one work context by its client surface and window identity.
    fn find_project_work_context(
        &self,
        surface: ProjectWorkContextSurface,
        window_id: &str,
    ) -> Result<Option<ProjectWorkContext>, RepositoryError>;

    /// Loads one active work context for the requested project if any non-expired row exists.
    fn find_active_project_work_context_for_project(
        &self,
        project_id: &ProjectId,
        active_after: i64,
    ) -> Result<Option<ProjectWorkContext>, RepositoryError>;

    /// Persists a replacement work context snapshot identified by its stable id.
    fn update_project_work_context(
        &self,
        context: ProjectWorkContext,
    ) -> Result<ProjectWorkContext, RepositoryError>;

    /// Deletes one work context by surface and window identity and reports whether it existed.
    fn delete_project_work_context(
        &self,
        surface: ProjectWorkContextSurface,
        window_id: &str,
    ) -> Result<bool, RepositoryError>;

    /// Deletes expired rows older than the supplied retention cutoff and returns the row count.
    fn delete_expired_project_work_contexts(
        &self,
        expired_before: i64,
    ) -> Result<usize, RepositoryError>;
}

/// Supplies new project work context identifiers for create flows.
pub trait ProjectWorkContextIdGenerator {
    /// Produces the identifier for a newly created project work context.
    fn generate_project_work_context_id(&self) -> ProjectWorkContextId;
}

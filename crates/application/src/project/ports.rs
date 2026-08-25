use crate::{BoxRepositorySource, RepositoryError};
use ora_domain::{Project, ProjectId, WorkspaceLocation};
use std::path::Path;
use thiserror::Error;

/// Describes one logical branch and the exact local ref Git should resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchReference {
    pub name: String,
    pub ref_name: String,
}

/// Supplies local Git branches without coupling project use cases to a Git implementation.
///
/// Implementations are expected to return resolvable local ref names without changing
/// repository state while exposing branches to the application layer.
pub trait BranchLister {
    /// Lists selectable branch references for the repository rooted at the supplied path.
    fn list_branches(
        &self,
        repository_root: &Path,
    ) -> Result<Vec<BranchReference>, BranchListingError>;
}

/// Supplies application-owned persistence operations for project CRUD use cases.
///
/// Implementations are expected to hide storage details such as soft-delete columns
/// while preserving the transport-agnostic behavior required by the handlers.
pub trait ProjectRepository {
    /// Persists a newly created project and returns the stored snapshot.
    fn create_project(
        &self,
        project: Project,
        main_workspace_location: WorkspaceLocation,
    ) -> Result<Project, RepositoryError>;

    /// Loads one visible project by identifier.
    fn find_project(&self, project_id: &ProjectId) -> Result<Option<Project>, RepositoryError>;

    /// Lists every visible project in storage order.
    fn list_projects(&self) -> Result<Vec<Project>, RepositoryError>;

    /// Persists a project replacement produced by the application layer.
    fn update_project(&self, project: Project) -> Result<Project, RepositoryError>;

    /// Marks a project deleted and returns whether a visible project was affected.
    fn soft_delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError>;
}

/// Supplies new project identifiers for create use cases.
pub trait ProjectIdGenerator {
    /// Produces the identifier for a newly created project.
    fn generate_project_id(&self) -> ProjectId;
}

/// Supplies the current timestamp in Unix milliseconds for application writes.
pub trait Clock {
    /// Returns the current Unix timestamp in milliseconds.
    fn now_timestamp_millis(&self) -> i64;
}

/// Captures branch-infrastructure failures that project handlers normalize for adapters.
#[derive(Debug, Error)]
pub enum BranchListingError {
    #[error("branch listing requires a Git repository")]
    NotARepository,
    #[error("branch listing operation failed")]
    OperationFailed(#[source] BoxRepositorySource),
}

impl BranchListingError {
    /// Wraps an infrastructure failure without flattening its diagnostic source chain.
    pub fn operation_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OperationFailed(Box::new(error))
    }
}

use crate::RepositoryError;
use ora_domain::{Project, ProjectId};

/// Supplies application-owned persistence operations for project CRUD use cases.
///
/// Implementations are expected to hide storage details such as soft-delete columns
/// while preserving the transport-agnostic behavior required by the handlers.
pub trait ProjectRepository {
    /// Persists a newly created project and returns the stored snapshot.
    fn create_project(&self, project: Project) -> Result<Project, RepositoryError>;

    /// Loads one visible project by identifier.
    fn find_project(&self, project_id: &ProjectId) -> Result<Option<Project>, RepositoryError>;

    /// Loads one visible project by its exact persisted name.
    fn find_project_by_name(&self, project_name: &str) -> Result<Option<Project>, RepositoryError>;

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

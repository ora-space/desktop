use crate::RepositoryError;
use ora_domain::{ProjectId, ProjectSpecSourceOverride, ProjectSpecSourceOverrideId};

/// Persists project-wide specification source decisions as one atomic collection.
///
/// Implementations must soft-delete the previous active rows and insert the supplied replacement in
/// one transaction so concurrent readers never observe a partially updated configuration.
pub trait ProjectSpecSourceOverrideRepository {
    /// Lists every active override owned by the supplied project.
    fn list_spec_source_overrides(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectSpecSourceOverride>, RepositoryError>;

    /// Atomically replaces all active overrides and returns the stored collection.
    fn replace_spec_source_overrides(
        &self,
        project_id: &ProjectId,
        replacements: Vec<ProjectSpecSourceOverride>,
        replaced_at: i64,
    ) -> Result<Vec<ProjectSpecSourceOverride>, RepositoryError>;
}

/// Supplies identifiers for newly persisted specification source overrides.
pub trait ProjectSpecSourceOverrideIdGenerator {
    /// Produces one independent override identifier.
    fn generate_spec_source_override_id(&self) -> ProjectSpecSourceOverrideId;
}

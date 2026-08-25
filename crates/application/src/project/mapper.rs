use ora_contracts::Project as ContractProject;
use ora_domain::Project as DomainProject;

/// Maps a domain project into the app-facing contract shape.
pub(crate) fn map_project(project: DomainProject) -> ContractProject {
    ContractProject {
        id: project.id.to_string(),
        name: project.name,
    }
}

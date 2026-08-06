use super::{ProjectSpecSourceOverrideIdGenerator, ProjectSpecSourceOverrideRepository};
use crate::{ApplicationError, Clock, ProjectRepository};
use ora_contracts::{
    ProjectSpecSourceOverride as ContractOverride, SpecSourceVisibility as ContractVisibility,
    SpecWorkflow as ContractWorkflow, UpdateProjectSpecSourcesRequest,
    UpdateProjectSpecSourcesResponse,
};
use ora_domain::{
    AuditFields, ProjectId, ProjectSpecSourceOverride, ProjectSpecSourceOverrideId,
    SpecSourceVisibility, SpecWorkflow,
};
use std::collections::BTreeSet;
use std::path::{Component, Path};
use uuid::Uuid;

/// Generates random identifiers for persisted project specification source overrides.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidProjectSpecSourceOverrideIdGenerator;

impl ProjectSpecSourceOverrideIdGenerator for UuidProjectSpecSourceOverrideIdGenerator {
    fn generate_spec_source_override_id(&self) -> ProjectSpecSourceOverrideId {
        ProjectSpecSourceOverrideId::new(Uuid::new_v4().to_string())
    }
}

/// Loads project-level source overrides for backend catalog composition.
pub struct ListProjectSpecSourceOverridesHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListProjectSpecSourceOverridesHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListProjectSpecSourceOverridesHandler<Repository>
where
    Repository: ProjectSpecSourceOverrideRepository,
{
    /// Returns the domain collection without introducing a filesystem-specific contract.
    pub fn handle(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectSpecSourceOverride>, ApplicationError> {
        self.repository
            .list_spec_source_overrides(project_id)
            .map_err(ApplicationError::from_spec_source_repository_error)
    }
}

/// Validates and atomically replaces one project's specification source overrides.
pub struct UpdateProjectSpecSourcesHandler<SourceRepository, ProjectStore, IdGenerator, ClockSource>
{
    source_repository: SourceRepository,
    project_repository: ProjectStore,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<SourceRepository, ProjectStore, IdGenerator, ClockSource>
    UpdateProjectSpecSourcesHandler<SourceRepository, ProjectStore, IdGenerator, ClockSource>
{
    pub fn new(
        source_repository: SourceRepository,
        project_repository: ProjectStore,
        id_generator: IdGenerator,
        clock: ClockSource,
    ) -> Self {
        Self {
            source_repository,
            project_repository,
            id_generator,
            clock,
        }
    }
}

impl<SourceRepository, ProjectStore, IdGenerator, ClockSource>
    UpdateProjectSpecSourcesHandler<SourceRepository, ProjectStore, IdGenerator, ClockSource>
where
    SourceRepository: ProjectSpecSourceOverrideRepository,
    ProjectStore: ProjectRepository,
    IdGenerator: ProjectSpecSourceOverrideIdGenerator,
    ClockSource: Clock,
{
    /// Replaces the entire active source configuration after validating project and path invariants.
    pub fn handle(
        &self,
        request: UpdateProjectSpecSourcesRequest,
    ) -> Result<UpdateProjectSpecSourcesResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let project_exists = self
            .project_repository
            .find_project(&project_id)
            .map_err(ApplicationError::from_project_repository_error)?
            .is_some();
        if !project_exists {
            return Err(ApplicationError::ProjectNotFound {
                project_id: project_id.to_string(),
            });
        }

        let now = self.clock.now_timestamp_millis();
        let mut seen_paths = BTreeSet::new();
        let mut replacements = Vec::with_capacity(request.sources.len());
        for source in request.sources {
            let relative_path = normalize_relative_path(&source.relative_path)?;
            if !seen_paths.insert(source_path_key(&relative_path)) {
                return Err(ApplicationError::SpecSourceInvalid);
            }
            let workflow = map_workflow(source.workflow)?;
            let visibility = map_visibility(source.visibility);
            replacements.push(ProjectSpecSourceOverride::new(
                self.id_generator.generate_spec_source_override_id(),
                project_id.clone(),
                relative_path,
                workflow,
                visibility,
                AuditFields::new(now, now, false),
            ));
        }

        let stored = self
            .source_repository
            .replace_spec_source_overrides(&project_id, replacements, now)
            .map_err(ApplicationError::from_spec_source_repository_error)?;

        Ok(UpdateProjectSpecSourcesResponse {
            sources: stored.into_iter().map(map_override).collect(),
        })
    }
}

/// Normalizes a portable workspace-relative path and rejects any escape or absolute component.
fn normalize_relative_path(value: &str) -> Result<String, ApplicationError> {
    let normalized_input = value.replace('\\', "/");
    let path = Path::new(&normalized_input);
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => segments.push(segment.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ApplicationError::SpecSourceInvalid);
            }
        }
    }
    if segments.is_empty() {
        return Err(ApplicationError::SpecSourceInvalid);
    }
    Ok(segments.join("/"))
}

/// Maps the tagged public workflow while enforcing the custom-name invariant at the application edge.
fn map_workflow(workflow: ContractWorkflow) -> Result<SpecWorkflow, ApplicationError> {
    match workflow {
        ContractWorkflow::OpenSpec => Ok(SpecWorkflow::OpenSpec),
        ContractWorkflow::Superpowers => Ok(SpecWorkflow::Superpowers),
        ContractWorkflow::Custom { name } if !name.trim().is_empty() => Ok(SpecWorkflow::Custom {
            name: name.trim().to_string(),
        }),
        ContractWorkflow::Custom { .. } => Err(ApplicationError::SpecSourceInvalid),
    }
}

/// Uses the host filesystem's case semantics when rejecting duplicate source paths.
fn source_path_key(relative_path: &str) -> String {
    if cfg!(windows) {
        relative_path.to_lowercase()
    } else {
        relative_path.to_string()
    }
}

/// Converts public visibility into the persistence-owned enum.
fn map_visibility(visibility: ContractVisibility) -> SpecSourceVisibility {
    match visibility {
        ContractVisibility::Enabled => SpecSourceVisibility::Enabled,
        ContractVisibility::Disabled => SpecSourceVisibility::Disabled,
    }
}

/// Projects one stored source override back into the public replacement response.
fn map_override(source: ProjectSpecSourceOverride) -> ContractOverride {
    ContractOverride {
        relative_path: source.relative_path,
        workflow: match source.workflow {
            SpecWorkflow::OpenSpec => ContractWorkflow::OpenSpec,
            SpecWorkflow::Superpowers => ContractWorkflow::Superpowers,
            SpecWorkflow::Custom { name } => ContractWorkflow::Custom { name },
        },
        visibility: match source.visibility {
            SpecSourceVisibility::Enabled => ContractVisibility::Enabled,
            SpecSourceVisibility::Disabled => ContractVisibility::Disabled,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{map_workflow, normalize_relative_path, source_path_key};
    use crate::ApplicationError;
    use ora_contracts::SpecWorkflow as ContractWorkflow;
    use ora_domain::SpecWorkflow;
    use pretty_assertions::assert_eq;

    /// Verifies persisted source paths use slash separators and reject workspace escapes.
    #[test]
    fn normalizes_safe_relative_paths() {
        assert_eq!(
            normalize_relative_path("docs\\specs\\api").unwrap(),
            "docs/specs/api"
        );
        assert_eq!(
            normalize_relative_path("../outside").unwrap_err(),
            ApplicationError::SpecSourceInvalid
        );
        assert_eq!(
            normalize_relative_path("./docs//specs/").unwrap(),
            "docs/specs"
        );
    }

    /// Verifies custom names are trimmed and blank names remain unrepresentable.
    #[test]
    fn validates_custom_workflow_names() {
        assert_eq!(
            map_workflow(ContractWorkflow::Custom {
                name: " Architecture ".to_string(),
            })
            .unwrap(),
            SpecWorkflow::Custom {
                name: "Architecture".to_string(),
            }
        );
        assert_eq!(
            map_workflow(ContractWorkflow::Custom {
                name: "  ".to_string(),
            })
            .unwrap_err(),
            ApplicationError::SpecSourceInvalid
        );
    }

    /// Verifies duplicate identity follows the host filesystem rather than a universal lowercase rule.
    #[test]
    fn keys_source_paths_with_host_case_semantics() {
        if cfg!(windows) {
            assert_eq!(source_path_key("Docs/Specs"), source_path_key("docs/specs"));
        } else {
            assert_ne!(source_path_key("Docs/Specs"), source_path_key("docs/specs"));
        }
    }
}

#[cfg(test)]
mod tests;

use ora_application::{
    AgentDefinitionRepository, FilesystemSkillStorage, FrozenSkillBinding, NodeType,
    RepositoryError, SkillRepository, StartPrerequisitesError, WorkflowGraph, WorkflowGraphNode,
    WorkflowRunWorkspaceInitializer, WorkflowSkillReceipt, skill_package_is_usable,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository, SqliteSkillRepository};
use ora_domain::{AgentDefinitionId, Namespace, Skill, SkillId, SkillOrigin};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Live Skill catalog resolution shared between precheck/start freezing and node dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedCatalogSkill {
    pub(super) catalog_skill_id: String,
    pub(super) catalog_name: String,
    pub(super) invocation_name: String,
    pub(super) skill_md_path: PathBuf,
}

/// Validates a run's roles and skill catalog bindings before creating, starting, or resuming it.
///
/// Roles and skills are deploy hard-dependencies: every agent's role must resolve in the agents
/// catalog and every enabled skill must resolve in Ora's catalog with a usable `SKILL.md`
/// package in formal storage. No skill directories are materialized inside the workspace; runtime
/// prompt delivery points the Agent directly at the storage `SKILL.md` path.
#[derive(Clone)]
pub struct SkillRoleWorkspaceInitializer {
    skills_root: PathBuf,
    pool: RepositoryPool,
}

impl SkillRoleWorkspaceInitializer {
    /// Creates an initializer backed by Ora's formal skills root and SQLite catalog.
    pub(crate) fn new(skills_root: PathBuf, pool: RepositoryPool) -> Self {
        Self { skills_root, pool }
    }

    /// Validates all reachable roles and skills and freezes catalog bindings without touching the
    /// workspace filesystem.
    pub(super) fn resolve_graph(
        &self,
        graph: &WorkflowGraph,
    ) -> Result<WorkflowSkillReceipt, StartPrerequisitesError> {
        self.resolve_graph_with_frozen(graph, /*frozen_receipt*/ None)
    }

    /// Re-validates a frozen run's graph while enforcing any already-frozen exact catalog IDs so
    /// start, restart, and same-snapshot resume never silently switch to another same-named skill.
    pub(super) fn validate_frozen_graph(
        &self,
        graph: &WorkflowGraph,
        frozen_receipt: &WorkflowSkillReceipt,
    ) -> Result<WorkflowSkillReceipt, StartPrerequisitesError> {
        self.resolve_graph_with_frozen(graph, Some(frozen_receipt))
    }

    fn resolve_graph_with_frozen(
        &self,
        graph: &WorkflowGraph,
        frozen_receipt: Option<&WorkflowSkillReceipt>,
    ) -> Result<WorkflowSkillReceipt, StartPrerequisitesError> {
        let roles = collect_roles(graph);
        let agent_repository = SqliteAgentDefinitionRepository::new(self.pool.clone());
        for role_id in &roles {
            if resolve_role(&agent_repository, role_id)?.is_none() {
                return Err(StartPrerequisitesError::WorkflowRoleNotFound {
                    role_id: role_id.clone(),
                });
            }
        }
        let skill_repository = SqliteSkillRepository::new(self.pool.clone());
        resolve_graph_skill_bindings(&self.skills_root, &skill_repository, graph, frozen_receipt)
    }
}

impl WorkflowRunWorkspaceInitializer for SkillRoleWorkspaceInitializer {
    fn initialize_workspace(
        &self,
        graph: &WorkflowGraph,
        _workspace_root: &Path,
    ) -> Result<WorkflowSkillReceipt, StartPrerequisitesError> {
        self.resolve_graph(graph)
    }
}

fn resolve_role(
    agent_repository: &SqliteAgentDefinitionRepository,
    role_id: &str,
) -> Result<Option<ora_domain::AgentDefinition>, RepositoryError> {
    let by_name = agent_repository.find_agent_definition_by_name(&Namespace::local(), role_id)?;
    if by_name.is_some() {
        return Ok(by_name);
    }
    agent_repository.find_agent_definition(&AgentDefinitionId::new(role_id))
}

/// Collects the distinct role ids declared across all agent nodes.
fn collect_roles(graph: &WorkflowGraph) -> Vec<String> {
    let mut roles = Vec::new();
    for scope in graph.execution_scopes() {
        for node in scope.nodes() {
            if node.node_type != NodeType::Agent {
                continue;
            }
            if let Some(config) = &node.agent_config
                && let Some(role_id) = &config.role_id
                && !role_id.trim().is_empty()
                && !roles.contains(role_id)
            {
                roles.push(role_id.clone());
            }
        }
    }
    roles
}

/// Validates every enabled skill in reachable agent nodes against the live catalog and freezes
/// immutable `{ node_id, skill_id, catalog_skill_id, invocation_name }` bindings.
fn resolve_graph_skill_bindings(
    skills_root: &Path,
    skill_repository: &SqliteSkillRepository,
    graph: &WorkflowGraph,
    frozen_receipt: Option<&WorkflowSkillReceipt>,
) -> Result<WorkflowSkillReceipt, StartPrerequisitesError> {
    let mut receipt = WorkflowSkillReceipt::default();
    let mut resolved_invocations = HashMap::<String, (String, String)>::new();
    for node in graph
        .execution_scopes()
        .into_iter()
        .flat_map(WorkflowGraph::nodes)
        .filter(|node| node.node_type == NodeType::Agent)
    {
        let Some(config) = &node.agent_config else {
            continue;
        };
        let frozen_node_bindings =
            frozen_receipt.map(|existing| existing.bindings_for_node(&node.id));
        let mut seen_skill_ids = HashSet::new();
        for skill in config.skills.iter().filter(|skill| skill.enabled) {
            if !seen_skill_ids.insert(skill.skill_id.clone()) {
                continue;
            }
            if skill.skill_id.trim().is_empty() {
                return Err(StartPrerequisitesError::WorkflowSkillNotFound {
                    node_id: node.id.clone(),
                    node_title: node_display_name(node),
                    skill_id: skill.skill_id.clone(),
                });
            }
            let frozen_catalog_id = frozen_node_bindings.as_ref().and_then(|bindings| {
                bindings
                    .iter()
                    .find(|binding| binding.skill_id == skill.skill_id)
                    .and_then(|binding| binding.catalog_skill_id.as_deref())
            });
            let resolved = match frozen_catalog_id {
                Some(catalog_skill_id) => resolve_exact_catalog_skill_entry(
                    skills_root,
                    skill_repository,
                    catalog_skill_id,
                )?,
                None => {
                    resolve_graph_ref_skill_entry(skills_root, skill_repository, &skill.skill_id)
                        .map_err(|error| match error {
                            StartPrerequisitesError::SkillDependencyError { message } => {
                                StartPrerequisitesError::SkillDependencyError {
                                    message: format!(
                                        "node {} ({}): {message}",
                                        node_display_name(node),
                                        node.id
                                    ),
                                }
                            }
                            other => other,
                        })?
                }
            };
            let Some(resolved) = resolved else {
                return Err(StartPrerequisitesError::WorkflowSkillNotFound {
                    node_id: node.id.clone(),
                    node_title: node_display_name(node),
                    skill_id: skill.skill_id.clone(),
                });
            };
            if let Some((existing_id, existing_name)) =
                resolved_invocations.get(&resolved.invocation_name)
                && existing_id != &resolved.catalog_skill_id
            {
                return Err(StartPrerequisitesError::SkillDependencyError {
                    message: format!(
                        "skills {existing_name} ({existing_id}) and {} ({}) resolve to the same prompt invocation /{}",
                        resolved.catalog_name, resolved.catalog_skill_id, resolved.invocation_name
                    ),
                });
            }
            resolved_invocations
                .entry(resolved.invocation_name.clone())
                .or_insert_with(|| {
                    (
                        resolved.catalog_skill_id.clone(),
                        resolved.catalog_name.clone(),
                    )
                });
            receipt.bindings.push(FrozenSkillBinding {
                node_id: node.id.clone(),
                skill_id: skill.skill_id.clone(),
                catalog_skill_id: Some(resolved.catalog_skill_id),
                invocation_name: resolved.invocation_name,
            });
        }
    }
    Ok(receipt)
}

/// Resolves one frozen `FrozenSkillBinding` for node dispatch.
///
/// Bindings that froze `catalog_skill_id` resolve strictly by that exact ID so an uninstalled
/// skill never falls back to a same-named skill. Legacy pre-Effect-retirement receipts without
/// `catalog_skill_id` resolve through the graph-reference compatibility path.
pub(super) fn resolve_binding_skill_entry(
    skills_root: &Path,
    skill_repository: &SqliteSkillRepository,
    binding: &FrozenSkillBinding,
) -> Result<Option<ResolvedCatalogSkill>, StartPrerequisitesError> {
    match binding.catalog_skill_id.as_deref() {
        Some(catalog_skill_id) => {
            resolve_exact_catalog_skill_entry(skills_root, skill_repository, catalog_skill_id)
        }
        None => resolve_graph_ref_skill_entry(skills_root, skill_repository, &binding.skill_id),
    }
}

/// Resolves one frozen catalog skill strictly by its exact catalog ID without name fallback.
///
/// Uninstalled, soft-deleted, or corrupt catalog entries return `Ok(None)` even if another active
/// skill shares the same name.
pub(super) fn resolve_exact_catalog_skill_entry(
    skills_root: &Path,
    skill_repository: &SqliteSkillRepository,
    catalog_skill_id: &str,
) -> Result<Option<ResolvedCatalogSkill>, StartPrerequisitesError> {
    let storage = FilesystemSkillStorage::new(skills_root.to_path_buf());
    let Some(exact) = skill_repository
        .find_skill(&SkillId::new(catalog_skill_id))
        .map_err(StartPrerequisitesError::Repository)?
    else {
        return Ok(None);
    };
    usable_entry(&storage, skills_root, &exact)
}

/// Resolves a graph-declared skill reference or legacy binding lacking `catalog_skill_id`.
///
/// Exact catalog `id` matches (including soft-deleted rows) and `plugin:` canonical IDs never fall
/// back to matching another skill by name. Only portable bare names written by the editor or
/// historical non-plugin `ns:name` suffixes match active catalog rows by name.
pub(super) fn resolve_graph_ref_skill_entry(
    skills_root: &Path,
    skill_repository: &SqliteSkillRepository,
    skill_id: &str,
) -> Result<Option<ResolvedCatalogSkill>, StartPrerequisitesError> {
    let storage = FilesystemSkillStorage::new(skills_root.to_path_buf());
    if let Some(exact) = skill_repository
        .find_skill_including_deleted(&SkillId::new(skill_id))
        .map_err(StartPrerequisitesError::Repository)?
    {
        if exact.audit_fields.is_deleted {
            return Ok(None);
        }
        return usable_entry(&storage, skills_root, &exact);
    }

    if skill_id.starts_with("plugin:") {
        return Ok(None);
    }

    let candidate = skill_id.rsplit(':').next().unwrap_or(skill_id);
    let skills = skill_repository
        .list_skills()
        .map_err(StartPrerequisitesError::Repository)?;
    let mut matching: Vec<&Skill> = skills
        .iter()
        .filter(|skill| skill.name.eq_ignore_ascii_case(candidate))
        .collect();
    if matching.len() > 1 {
        matching.sort_by(|a, b| a.id.as_ref().cmp(b.id.as_ref()));
        let candidates = matching
            .iter()
            .map(|skill| format!("{} ({})", skill.id, skill.namespace))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(StartPrerequisitesError::SkillDependencyError {
            message: format!(
                "skill reference `{skill_id}` matches multiple skills ({candidates}); specify a concrete skill instead"
            ),
        });
    }
    match matching.into_iter().next() {
        Some(skill) => usable_entry(&storage, skills_root, skill),
        None => Ok(None),
    }
}

fn usable_entry(
    storage: &FilesystemSkillStorage,
    skills_root: &Path,
    skill: &Skill,
) -> Result<Option<ResolvedCatalogSkill>, StartPrerequisitesError> {
    let usable = skill_package_is_usable(storage, skill).map_err(|error| {
        StartPrerequisitesError::SkillDependencyError {
            message: error.to_string(),
        }
    })?;
    if !usable {
        return Ok(None);
    }
    Ok(Some(ResolvedCatalogSkill {
        catalog_skill_id: skill.id.to_string(),
        catalog_name: skill.name.clone(),
        invocation_name: normalize_skill_name(&skill.name),
        skill_md_path: storage_package_dir(skills_root, skill).join("SKILL.md"),
    }))
}

fn storage_package_dir(skills_root: &Path, skill: &Skill) -> PathBuf {
    match &skill.origin {
        SkillOrigin::Local => {
            ora_utils::path::strip_verbatim_prefix(&skills_root.join(&skill.name))
        }
        SkillOrigin::Plugin { package_root, .. } => {
            ora_utils::path::strip_verbatim_prefix(package_root)
        }
    }
}

/// Returns the label a dependency failure should show for one node.
pub(super) fn node_display_name(node: &WorkflowGraphNode) -> String {
    let title = node.title.trim();
    if title.is_empty() {
        node.id.clone()
    } else {
        title.to_string()
    }
}

/// Normalizes a catalog name into its lowercase hyphenated workflow label.
fn normalize_skill_name(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

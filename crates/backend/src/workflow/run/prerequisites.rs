#[cfg(test)]
mod tests;

use ora_application::{
    AgentDefinitionRepository, FilesystemSkillStorage, MaterializedSkillBinding, NodeType,
    RepositoryError, SkillMaterializationReceipt, SkillRepository, StartPrerequisitesError,
    WorkflowGraph, WorkflowGraphNode, WorkflowRunWorkspaceInitializer, skill_package_is_usable,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository, SqliteSkillRepository};
use ora_domain::{AgentDefinitionId, Namespace, Skill, SkillId, SkillOrigin};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Live Skill catalog resolution shared between precheck/start freezing and node dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedCatalogSkill {
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
    ) -> Result<SkillMaterializationReceipt, StartPrerequisitesError> {
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
        resolve_graph_skill_bindings(&self.skills_root, &skill_repository, graph)
    }
}

impl WorkflowRunWorkspaceInitializer for SkillRoleWorkspaceInitializer {
    fn initialize_workspace(
        &self,
        graph: &WorkflowGraph,
        _workspace_root: &Path,
    ) -> Result<SkillMaterializationReceipt, StartPrerequisitesError> {
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
/// immutable `{ node_id, skill_id, invocation_name }` bindings.
fn resolve_graph_skill_bindings(
    skills_root: &Path,
    skill_repository: &SqliteSkillRepository,
    graph: &WorkflowGraph,
) -> Result<SkillMaterializationReceipt, StartPrerequisitesError> {
    let mut receipt = SkillMaterializationReceipt::default();
    for node in graph
        .execution_scopes()
        .into_iter()
        .flat_map(WorkflowGraph::nodes)
        .filter(|node| node.node_type == NodeType::Agent)
    {
        let Some(config) = &node.agent_config else {
            continue;
        };
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
            let Some(resolved) =
                resolve_live_skill_entry(skills_root, skill_repository, &skill.skill_id)?
            else {
                return Err(StartPrerequisitesError::WorkflowSkillNotFound {
                    node_id: node.id.clone(),
                    node_title: node_display_name(node),
                    skill_id: skill.skill_id.clone(),
                });
            };
            receipt.bindings.push(MaterializedSkillBinding {
                node_id: node.id.clone(),
                skill_id: skill.skill_id.clone(),
                invocation_name: resolved.invocation_name,
            });
        }
    }
    Ok(receipt)
}

/// Resolves one skill reference against Ora's live catalog and verifies its `SKILL.md` is usable.
///
/// Exact catalog `id` matches take precedence and are never shadowed or fallen back from when the
/// owned package is deleted or corrupt. When no row has that exact `id`, legacy bare/namespaced
/// suffixes match active catalog rows by name for compatibility with older graph snapshots.
pub(super) fn resolve_live_skill_entry(
    skills_root: &Path,
    skill_repository: &SqliteSkillRepository,
    skill_id: &str,
) -> Result<Option<ResolvedCatalogSkill>, StartPrerequisitesError> {
    let storage = FilesystemSkillStorage::new(skills_root.to_path_buf());
    if let Some(exact) = skill_repository
        .find_skill(&SkillId::new(skill_id))
        .map_err(StartPrerequisitesError::Repository)?
    {
        if exact.audit_fields.is_deleted {
            return Ok(None);
        }
        return usable_entry(&storage, skills_root, &exact);
    }

    let candidate = skill_id.rsplit(':').next().unwrap_or(skill_id);
    let skills = skill_repository
        .list_skills()
        .map_err(StartPrerequisitesError::Repository)?;
    for skill in skills
        .iter()
        .filter(|skill| skill.name.eq_ignore_ascii_case(candidate))
    {
        if let Some(entry) = usable_entry(&storage, skills_root, skill)? {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

fn usable_entry(
    storage: &FilesystemSkillStorage,
    skills_root: &Path,
    skill: &Skill,
) -> Result<Option<ResolvedCatalogSkill>, StartPrerequisitesError> {
    let usable = skill_package_is_usable(storage, skill).map_err(|error| {
        StartPrerequisitesError::SkillMaterializationError {
            message: error.to_string(),
        }
    })?;
    if !usable {
        return Ok(None);
    }
    Ok(Some(ResolvedCatalogSkill {
        invocation_name: normalize_skill_name(&skill.name),
        skill_md_path: storage_package_dir(skills_root, skill).join("SKILL.md"),
    }))
}

fn storage_package_dir(skills_root: &Path, skill: &Skill) -> PathBuf {
    match &skill.origin {
        SkillOrigin::Local => skills_root.join(&skill.name),
        SkillOrigin::Plugin { package_root, .. } => package_root.clone(),
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

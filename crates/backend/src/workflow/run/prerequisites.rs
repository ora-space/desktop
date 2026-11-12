use ora_application::{
    AgentDefinitionRepository, AgentSkillDelivery, AgentSkillDeliveryProvider,
    FilesystemSkillStorage, MaterializedSkillBinding, NodeType, RepositoryError,
    SkillDiscoveryRoots, SkillMaterializationReceipt, SkillRepository, StartPrerequisitesError,
    WorkflowGraph, WorkflowRunWorkspaceInitializer, has_usable_package, skill_package_is_usable,
};
use ora_db::{
    RepositoryPool, SqliteAgentDefinitionRepository, SqliteSkillRepository, TimestampSource,
};
use ora_domain::{AgentDefinitionId, AgentRef, Namespace, PluginId};
use ora_effect::{ConsumerDeclaration, SkillName};
use ora_effect_skill::materialized_skill_directory;
use ora_utils::path::StrictRelativePath;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Current host capability used until Agent plugins publish their own discovery roots.
///
/// Keeping the default behind [`AgentSkillDeliveryProvider`] means workflow validation and prompt
/// rendering consume the same placements that the Effect subsystem materializes.
#[derive(Clone)]
pub struct SharedAgentSkillDeliveryProvider {
    discovery_roots: SkillDiscoveryRoots,
}

impl SharedAgentSkillDeliveryProvider {
    /// Builds the current shared capability after validating its worktree-relative root.
    fn new() -> Result<Self, ora_application::AgentSkillDeliveryError> {
        let root = StrictRelativePath::parse(".agents/skills").map_err(|error| {
            ora_application::AgentSkillDeliveryError::Invalid {
                message: format!("invalid built-in shared skill root: {error:?}"),
            }
        })?;
        Ok(Self {
            discovery_roots: SkillDiscoveryRoots::new(root, Vec::new()),
        })
    }
}

impl AgentSkillDeliveryProvider for SharedAgentSkillDeliveryProvider {
    fn skill_delivery(
        &self,
        _agent_ref: &AgentRef,
    ) -> Result<AgentSkillDelivery, ora_application::AgentSkillDeliveryError> {
        Ok(AgentSkillDelivery::Filesystem {
            discovery_roots: self.discovery_roots.clone(),
        })
    }
}

/// Resolves discovery roots from agent plugin declarations, falling back to the
/// shared default when a plugin has not declared an Effect Resource.
#[derive(Clone)]
pub struct PluginAwareAgentSkillDeliveryProvider {
    plugin_api: Arc<crate::plugin::PluginApi>,
    fallback: SkillDiscoveryRoots,
}

impl PluginAwareAgentSkillDeliveryProvider {
    /// Builds a provider that reads discovery roots from live plugin declarations.
    pub fn new(
        plugin_api: Arc<crate::plugin::PluginApi>,
    ) -> Result<Self, ora_application::AgentSkillDeliveryError> {
        let fallback_root = StrictRelativePath::parse(".agents/skills").map_err(|error| {
            ora_application::AgentSkillDeliveryError::Invalid {
                message: format!("invalid built-in shared skill root: {error:?}"),
            }
        })?;
        Ok(Self {
            plugin_api,
            fallback: SkillDiscoveryRoots::new(fallback_root, Vec::new()),
        })
    }
}

impl AgentSkillDeliveryProvider for PluginAwareAgentSkillDeliveryProvider {
    fn skill_delivery(
        &self,
        agent_ref: &AgentRef,
    ) -> Result<AgentSkillDelivery, ora_application::AgentSkillDeliveryError> {
        // AgentRef.as_str() is the canonical plugin id (e.g. "official/my-agent")
        let declaration = PluginId::parse(agent_ref.as_str())
            .ok()
            .and_then(|plugin_id| self.plugin_api.agent_effect_declaration_for(&plugin_id));
        let discovery_roots = skill_roots_from_declaration(declaration.as_ref())?
            .unwrap_or_else(|| self.fallback.clone());
        Ok(AgentSkillDelivery::Filesystem { discovery_roots })
    }
}

/// Resolves the discovery roots one Consumer declaration contributes, or `None` when it declares no
/// Skill directory.
///
/// A declared Skill directory that cannot be spelled as a worktree-relative path is an error rather
/// than an absent declaration: falling back would aim the prompt at a directory the Effect
/// subsystem never materializes, which is the mismatch this provider exists to remove.
fn skill_roots_from_declaration(
    declaration: Option<&ConsumerDeclaration>,
) -> Result<Option<SkillDiscoveryRoots>, ora_application::AgentSkillDeliveryError> {
    let Some(declaration) = declaration else {
        return Ok(None);
    };
    let mut roots: Vec<StrictRelativePath> = Vec::new();
    for resource in &declaration.resources {
        if resource.materialization_format
            != ora_effect::MaterializationFormat::skill_directory_v1()
        {
            continue;
        }
        let declared = resource.relative_path.as_str();
        let root = StrictRelativePath::parse(declared).map_err(|error| {
            ora_application::AgentSkillDeliveryError::Invalid {
                message: format!(
                    "declared Skill discovery root {declared} is not a worktree-relative path: {error:?}"
                ),
            }
        })?;
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    let mut roots = roots.into_iter();
    let Some(first) = roots.next() else {
        return Ok(None);
    };
    Ok(Some(SkillDiscoveryRoots::new(first, roots.collect())))
}

/// Validates a run workspace's roles and skill bindings at deploy time.
///
/// Roles and skills are deploy hard-dependencies: every agent's role must resolve in the agents
/// catalog and every enabled skill must exist in the catalog with a loadable package — the formal
/// directory for a local skill, the immutable plugin package for a plugin-imported skill. The
/// Effect subsystem owns physical package materialization; this initializer only freezes the
/// invocation names and Effect-owned discovery paths that execution uses to build the prompt.
#[derive(Clone)]
pub struct SkillRoleWorkspaceInitializer<DeliveryProvider = SharedAgentSkillDeliveryProvider> {
    skills_root: PathBuf,
    pool: RepositoryPool,
    delivery_provider: DeliveryProvider,
}

impl SkillRoleWorkspaceInitializer<SharedAgentSkillDeliveryProvider> {
    /// Builds an initializer from the skill catalog root and the shared repository pool.
    pub fn new(
        skills_root: PathBuf,
        pool: RepositoryPool,
    ) -> Result<Self, ora_application::AgentSkillDeliveryError> {
        Ok(Self::with_delivery_provider(
            skills_root,
            pool,
            SharedAgentSkillDeliveryProvider::new()?,
        ))
    }
}

impl<DeliveryProvider> SkillRoleWorkspaceInitializer<DeliveryProvider> {
    /// Builds an initializer with an injected Agent capability provider.
    pub fn with_delivery_provider(
        skills_root: PathBuf,
        pool: RepositoryPool,
        delivery_provider: DeliveryProvider,
    ) -> Self {
        Self {
            skills_root,
            pool,
            delivery_provider,
        }
    }
}

impl<DeliveryProvider> WorkflowRunWorkspaceInitializer
    for SkillRoleWorkspaceInitializer<DeliveryProvider>
where
    DeliveryProvider: AgentSkillDeliveryProvider,
{
    fn initialize_workspace(
        &self,
        graph: &WorkflowGraph,
        _workspace_root: &Path,
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

        let storage = FilesystemSkillStorage::new(self.skills_root.clone());
        let skill_repository = SqliteSkillRepository::new(self.pool.clone());
        resolve_graph_skill_bindings(&storage, &skill_repository, &self.delivery_provider, graph)
    }
}

/// Resolves a role by name first, falling back to the agent definition id for graphs that stored
/// the id as `roleId` (the pre-empty-role editor did).
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
            let Some(config) = &node.agent_config else {
                continue;
            };
            if let Some(role_id) = &config.role_id
                && !role_id.trim().is_empty()
                && !roles.contains(role_id)
            {
                roles.push(role_id.clone());
            }
        }
    }
    roles
}

/// Resolves every node's enabled skills to the Effect-owned paths later consumed by execution.
fn resolve_graph_skill_bindings<DeliveryProvider>(
    storage: &FilesystemSkillStorage,
    skill_repository: &SqliteSkillRepository<impl TimestampSource>,
    delivery_provider: &DeliveryProvider,
    graph: &WorkflowGraph,
) -> Result<SkillMaterializationReceipt, StartPrerequisitesError>
where
    DeliveryProvider: AgentSkillDeliveryProvider,
{
    let mut receipt = SkillMaterializationReceipt::default();
    let mut resolved_packages = HashMap::<StrictRelativePath, String>::new();
    let mut resolved_invocations = HashMap::<StrictRelativePath, String>::new();
    for node in graph
        .execution_scopes()
        .into_iter()
        .flat_map(WorkflowGraph::nodes)
        .filter(|node| node.node_type == NodeType::Agent)
    {
        let Some(config) = &node.agent_config else {
            continue;
        };
        let enabled_skills = config
            .skills
            .iter()
            .filter(|skill| skill.enabled)
            .collect::<Vec<_>>();
        if enabled_skills.is_empty() {
            continue;
        }
        let agent_ref = AgentRef::parse(&config.executor.agent_cli).map_err(|error| {
            StartPrerequisitesError::AgentSkillDeliveryError {
                agent_ref: config.executor.agent_cli.clone(),
                message: error.to_string(),
            }
        })?;
        let delivery = delivery_provider
            .skill_delivery(&agent_ref)
            .map_err(|error| StartPrerequisitesError::AgentSkillDeliveryError {
                agent_ref: config.executor.agent_cli.clone(),
                message: error.to_string(),
            })?;
        let AgentSkillDelivery::Filesystem { discovery_roots } = delivery else {
            return Err(StartPrerequisitesError::AgentSkillDeliveryUnsupported {
                agent_ref: config.executor.agent_cli.clone(),
            });
        };
        let mut seen_skill_ids = HashSet::new();
        for skill in enabled_skills {
            if !seen_skill_ids.insert(skill.skill_id.clone()) {
                continue;
            }
            let catalog_name =
                resolve_skill_catalog_name(storage, skill_repository, &skill.skill_id)?;
            let invocation_name = normalize_skill_name(&catalog_name);
            // Distinct catalog names can collapse onto one prompt invocation within the same
            // discovery root even when their canonical on-disk directories stay distinct. Keep
            // the previous root-plus-invocation conflict key so different roots stay independent.
            for root in discovery_roots.iter() {
                let invocation_path = root.append_segment(&invocation_name);
                if let Some(existing_catalog_name) = resolved_invocations.get(&invocation_path)
                    && existing_catalog_name != &catalog_name
                {
                    return Err(StartPrerequisitesError::SkillMaterializationError {
                        message: format!(
                            "skills {existing_catalog_name} and {catalog_name} resolve to the same prompt invocation /{invocation_name} under discovery root {root}"
                        ),
                    });
                }
                resolved_invocations
                    .entry(invocation_path)
                    .or_insert_with(|| catalog_name.clone());
            }
            // Effect materializes the canonical `SkillName`, which lowercases ASCII but keeps `_`.
            // The prompt's `/name` invocation spelling above is separate and may use `-`.
            let materialized_name = SkillName::parse(&catalog_name).map_err(|error| {
                StartPrerequisitesError::SkillMaterializationError {
                    message: format!(
                        "catalog skill name {catalog_name} is not a valid Skill name: {error}"
                    ),
                }
            })?;
            let package_paths = discovery_roots
                .iter()
                .map(|root| root.append_segment(materialized_skill_directory(&materialized_name)))
                .collect::<Vec<_>>();
            for package_path in &package_paths {
                if let Some(existing_catalog_name) = resolved_packages.get(package_path) {
                    if existing_catalog_name != &catalog_name {
                        return Err(StartPrerequisitesError::SkillMaterializationError {
                            message: format!(
                                "skills {existing_catalog_name} and {catalog_name} resolve to the same worktree path {package_path}"
                            ),
                        });
                    }
                    continue;
                }
                resolved_packages.insert(package_path.clone(), catalog_name.clone());
            }
            receipt.bindings.push(MaterializedSkillBinding {
                node_id: node.id.clone(),
                skill_id: skill.skill_id.clone(),
                invocation_name,
                package_paths,
            });
        }
    }
    Ok(receipt)
}

/// Resolves one enabled skill id to its catalog name.
///
/// Graphs store the catalog name in `skillId`, so the candidate name first resolves through the
/// local formal package directory. When that directory is absent — the shape of every
/// plugin-imported skill, whose package stays inside the plugin installation — the catalog row is
/// matched and usability follows the row's owning package: the formal directory for a Local skill
/// and the immutable plugin package for a Plugin skill, mirroring the availability that the
/// settings and editor surfaces display for the same skill. A legacy namespaced id like
/// `cdase:sfmea_review` resolves by the suffix after the colon, and a full catalog id such as
/// `plugin:<plugin_id>:<name>` claims its row before the name match.
fn resolve_skill_catalog_name(
    storage: &FilesystemSkillStorage,
    skill_repository: &SqliteSkillRepository<impl TimestampSource>,
    skill_id: &str,
) -> Result<String, StartPrerequisitesError> {
    let candidate = skill_id.rsplit(':').next().unwrap_or(skill_id);
    if skill_package_usable(storage, candidate)? {
        return Ok(candidate.to_string());
    }
    let skills = skill_repository
        .list_skills()
        .map_err(StartPrerequisitesError::Repository)?;
    // An exact catalog id match wins over name matches so a stored row id is honored even when
    // its suffix is not the catalog name; name matching is ASCII-case-insensitive like the
    // repository's COLLATE NOCASE lookups.
    let mut matched = Vec::new();
    if let Some(by_id) = skills.iter().find(|skill| skill.id.as_ref() == skill_id) {
        matched.push(by_id);
    }
    matched.extend(skills.iter().filter(|skill| {
        skill.name.eq_ignore_ascii_case(candidate) && skill.id.as_ref() != skill_id
    }));
    for skill in matched {
        let usable = skill_package_is_usable(storage, skill).map_err(|error| {
            StartPrerequisitesError::SkillMaterializationError {
                message: error.to_string(),
            }
        })?;
        if usable {
            return Ok(skill.name.clone());
        }
    }
    Err(StartPrerequisitesError::WorkflowSkillNotFound {
        skill_id: skill_id.to_string(),
    })
}

/// Returns whether the local catalog still has a formal package that parses as a skill manifest.
fn skill_package_usable(
    storage: &FilesystemSkillStorage,
    name: &str,
) -> Result<bool, StartPrerequisitesError> {
    has_usable_package(storage, name).map_err(|error| {
        StartPrerequisitesError::SkillMaterializationError {
            message: error.to_string(),
        }
    })
}

/// Resolves an enabled skill id to the executable `/name` spelling the prompt presents.
///
/// This Agent-facing invocation spelling is separate from the materialized directory identity, so
/// it may normalize `_` to `-` even though the package stays at its canonical on-disk name.
#[cfg(test)]
fn resolve_executable_skill_name(
    storage: &FilesystemSkillStorage,
    skill_repository: &SqliteSkillRepository<impl TimestampSource>,
    skill_id: &str,
) -> Result<String, StartPrerequisitesError> {
    Ok(normalize_skill_name(&resolve_skill_catalog_name(
        storage,
        skill_repository,
        skill_id,
    )?))
}

/// Normalizes a catalog name for the prompt's Agent-facing `/name` invocation: lowercase, `_`
/// becomes `-`.
///
/// This is deliberately not the materialized directory identity; use
/// [`ora_effect_skill::materialized_skill_directory`] for the package path.
fn normalize_skill_name(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::run::prompt::{
        RequiredWorkflowSkill, render_prompt_path, render_required_skills,
    };
    use ora_contracts::WorkflowRunLocale;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, default_migration_catalog,
    };
    use ora_domain::PluginId;
    use ora_effect::{
        Digest, EffectMutation, EffectResource, EffectResourceId, EffectScopeId, ExactPlannedState,
        ExactPreviousState, FilesystemDirectoryDescriptor, Fingerprint, Generation, LocalTimestamp,
        ManagedIdentity, MaterializationFormat, NativeResourceIdentity, PlannedMutation,
        ReconcileAttemptId, ResourceAdapter, ResourceAdapterIdentity, ResourceKey,
        ResourceLifecycle, ResourcePath, SkillMaterializationInput, SkillSourceKey,
        SkillSourceKind, VersionedAdapterPlan, VersionedMaterializationInput,
        VersionedResourceDescriptor,
    };
    use ora_effect_skill::SkillDirectoryResourceAdapter;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FixedDeliveryProvider {
        delivery: AgentSkillDelivery,
    }

    impl AgentSkillDeliveryProvider for FixedDeliveryProvider {
        fn skill_delivery(
            &self,
            _agent_ref: &AgentRef,
        ) -> Result<AgentSkillDelivery, ora_application::AgentSkillDeliveryError> {
            Ok(self.delivery.clone())
        }
    }

    /// Supplies the delivery capability declared by each Agent in a multi-root test graph.
    #[derive(Clone)]
    struct AgentKeyedDeliveryProvider {
        deliveries: HashMap<String, AgentSkillDelivery>,
    }

    impl AgentSkillDeliveryProvider for AgentKeyedDeliveryProvider {
        fn skill_delivery(
            &self,
            agent_ref: &AgentRef,
        ) -> Result<AgentSkillDelivery, ora_application::AgentSkillDeliveryError> {
            self.deliveries
                .get(agent_ref.as_str())
                .cloned()
                .ok_or(ora_application::AgentSkillDeliveryError::Unavailable)
        }
    }

    /// Opens an isolated repository pool used by capability-driven binding tests.
    fn test_pool(temp: &TempDir) -> RepositoryPool {
        DatabaseBootstrapper::new(crate::test_clock::TestClock)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&temp.path().join("ora.sqlite3")),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool")
    }

    /// Projects one installed plugin skill into the catalog with its package kept outside the
    /// local skills root, mirroring how a plugin import publishes its immutable skills.
    fn install_plugin_skill(pool: &RepositoryPool, package_root: &Path, name: &str) {
        std::fs::create_dir_all(package_root).unwrap();
        std::fs::write(
            package_root.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: review\n---\n"),
        )
        .unwrap();
        SqliteSkillRepository::with_clock(pool.clone(), crate::test_clock::TestClock)
            .replace_plugin_skills(
                &PluginId::new("official", "review-pack").unwrap(),
                "1.2.3",
                &[PluginSkillProjection {
                    name: name.to_string(),
                    description: "Reviews changes".to_string(),
                    package_root: package_root.to_path_buf(),
                    skill_md_digest: ora_effect::Digest::sha256(b"manifest"),
                    package_fingerprint: ora_effect::Fingerprint::from(
                        ora_utils::directory::fingerprint_directory(package_root, &[]).unwrap(),
                    ),
                }],
                10,
            )
            .unwrap();
    }

    /// Materializes one Skill package through the production Effect adapter at `relative_root`.
    ///
    /// Returns the path the adapter actually wrote, so tests compare receipt paths with
    /// materialization instead of re-deriving the naming policy under test.
    fn materialize_skill_at(
        worktree: &Path,
        relative_root: &str,
        source: &Path,
        skill_name: &str,
    ) -> PathBuf {
        let typed_name = SkillName::parse(skill_name).unwrap();
        let resource = EffectResource {
            identity: EffectResourceId::new("resource-1"),
            scope: EffectScopeId::Workspace(ora_domain::WorkspaceId::new("workspace-1")),
            resource_key: ResourceKey::parse(format!("filesystem-directory:{relative_root}"))
                .unwrap(),
            adapter: ResourceAdapterIdentity::parse("ora/filesystem-directory").unwrap(),
            descriptor: VersionedResourceDescriptor::FilesystemDirectoryV1(
                FilesystemDirectoryDescriptor {
                    workspace_root: worktree.to_path_buf(),
                    relative_path: ResourcePath::parse(relative_root).unwrap(),
                },
            ),
            format: MaterializationFormat::skill_directory_v1(),
            lifecycle: ResourceLifecycle::Active,
        };
        let fingerprint =
            Fingerprint::from(ora_utils::directory::fingerprint_directory(source, &[]).unwrap());
        let managed_identity = ManagedIdentity::random();
        let native_identity =
            NativeResourceIdentity::parse(materialized_skill_directory(&typed_name)).unwrap();
        let operation = SkillDirectoryResourceAdapter
            .prepare_operation(
                &resource,
                ReconcileAttemptId::random(),
                Generation::new(1),
                /*sequence*/ 0,
                PlannedMutation {
                    managed_identity: managed_identity.clone(),
                    desired_effect: None,
                    mutation: EffectMutation::Create,
                    expected: ExactPreviousState::Missing,
                    planned: ExactPlannedState::Present {
                        native_identity: native_identity.clone(),
                        managed_identity,
                        fingerprint: fingerprint.clone(),
                    },
                    input: Some(VersionedMaterializationInput::SkillDirectoryV1(
                        SkillMaterializationInput {
                            name: typed_name.clone(),
                            source: SkillSourceKey {
                                source_kind: SkillSourceKind::Local,
                                namespace: Namespace::local(),
                                name: typed_name,
                            },
                            package_root: source.to_path_buf(),
                            skill_md_digest: Digest::sha256(b"manifest"),
                            package_fingerprint: fingerprint,
                        },
                    )),
                },
                LocalTimestamp::from_millis(1),
            )
            .unwrap()
            .operation;
        let VersionedAdapterPlan::FilesystemDirectoryV1(plan) = operation.payload() else {
            panic!("Skill adapter must plan a filesystem directory");
        };
        let materialized_path = plan.resource_root.join(native_identity.as_str());
        SkillDirectoryResourceAdapter.apply(&operation).unwrap();
        assert!(materialized_path.join("SKILL.md").is_file());
        materialized_path
    }

    /// Builds one declared Skill directory at `path` for declaration-resolution tests.
    fn declared_skill(path: &str) -> ora_effect::FilesystemResourceTemplate {
        ora_effect::FilesystemResourceTemplate {
            relative_path: ora_effect::ResourcePath::parse(path).expect("valid resource path"),
            materialization_format: ora_effect::MaterializationFormat::skill_directory_v1(),
            materialization_contract: ora_effect::MaterializationContract::skill_directory_v1(),
            accepts: ora_effect::CapabilityRequirement::default(),
            coordination: ora_effect::CoordinationRequirement::Uninterrupted,
            ownership_relative_path: None,
        }
    }

    /// Builds one Consumer declaration contributing exactly the given resources.
    fn declaration_of(
        resources: Vec<ora_effect::FilesystemResourceTemplate>,
    ) -> ora_effect::ConsumerDeclaration {
        ora_effect::ConsumerDeclaration {
            consumer: ora_effect::ConsumerIdentity::new(
                ora_effect::ConsumerKind::agent_plugin(),
                "official/review-pack",
            )
            .expect("valid consumer identity"),
            adapter: ora_effect::ConsumerAdapterIdentity::parse("ora/agent-plugin")
                .expect("valid adapter identity"),
            capabilities: ora_effect::CapabilitySet::default(),
            resources,
        }
    }

    /// An Agent whose plugin registered no Effect declaration keeps the shared discovery root.
    #[test]
    fn an_undeclared_plugin_keeps_the_shared_skill_root() {
        assert_eq!(skill_roots_from_declaration(None).unwrap(), None);
    }

    /// A declaration that contributes no Skill directory also keeps the shared discovery root.
    #[test]
    fn a_declaration_without_skill_resources_keeps_the_shared_skill_root() {
        let mut owned_file = declared_skill(".agents/skills");
        owned_file.materialization_format =
            ora_effect::MaterializationFormat::parse("ora/owned-file.v1").unwrap();
        owned_file.ownership_relative_path =
            Some(ora_effect::ResourcePath::parse(".agents/owned").unwrap());
        assert_eq!(
            skill_roots_from_declaration(Some(&declaration_of(vec![owned_file]))).unwrap(),
            None
        );
    }

    /// Every declared Skill directory becomes a root, preserving declaration order.
    #[test]
    fn declared_skill_directories_become_roots_in_declaration_order() {
        let declaration = declaration_of(vec![
            declared_skill(".cac/skills"),
            declared_skill(".opencode/skills"),
        ]);
        assert_eq!(
            skill_roots_from_declaration(Some(&declaration)).unwrap(),
            Some(SkillDiscoveryRoots::new(
                StrictRelativePath::parse(".cac/skills").unwrap(),
                vec![StrictRelativePath::parse(".opencode/skills").unwrap()],
            ))
        );
    }

    /// A declared root the worktree path type cannot represent fails instead of falling back: the
    /// Effect subsystem will materialize it, so a silent fallback would recreate the mismatch.
    #[test]
    fn an_unrepresentable_declared_skill_root_is_rejected() {
        // `ResourcePath` accepts this depth; the stricter worktree path stops at 32 levels.
        let deep = vec!["level"; 34].join("/");
        assert!(matches!(
            skill_roots_from_declaration(Some(&declaration_of(vec![declared_skill(&deep)]))),
            Err(ora_application::AgentSkillDeliveryError::Invalid { ref message })
                if message.contains("TooDeep")
        ));
    }

    #[test]
    fn normalizes_skill_names_to_lowercase_dashes() {
        assert_eq!(normalize_skill_name("sfmea_review"), "sfmea-review");
        assert_eq!(normalize_skill_name("OpenSpec_Explore"), "openspec-explore");
    }

    #[test]
    fn resolves_the_executable_skill_name_from_a_namespaced_id() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        std::fs::create_dir_all(skills_root.join("sfmea_review")).unwrap();
        std::fs::write(
            skills_root.join("sfmea_review").join("SKILL.md"),
            "---\nname: sfmea_review\ndescription: review\n---\n",
        )
        .unwrap();
        let storage = FilesystemSkillStorage::new(skills_root);
        // An empty catalog keeps this legacy-id resolution on the local package fast path.
        let catalog_temp = TempDir::new().unwrap();
        let repository = SqliteSkillRepository::with_clock(
            test_pool(&catalog_temp),
            crate::test_clock::TestClock,
        );
        assert_eq!(
            resolve_executable_skill_name(&storage, &repository, "cdase:sfmea_review").unwrap(),
            "sfmea-review"
        );
    }

    /// A plugin-imported skill resolves through its immutable plugin package — by the catalog
    /// name the editor stores in `skillId` and by its full `plugin:` catalog id — even though no
    /// formal directory exists in the local skills root.
    #[test]
    fn resolves_a_plugin_skill_through_its_plugin_package() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let package_root = temp.path().join("plugins/review-pack/assets/review");
        let pool = test_pool(&temp);
        install_plugin_skill(&pool, &package_root, "review");
        let storage = FilesystemSkillStorage::new(skills_root);
        let repository = SqliteSkillRepository::with_clock(pool, crate::test_clock::TestClock);

        assert_eq!(
            resolve_executable_skill_name(&storage, &repository, "review").unwrap(),
            "review"
        );
        assert_eq!(
            resolve_executable_skill_name(
                &storage,
                &repository,
                "plugin:official/review-pack:review",
            )
            .unwrap(),
            "review"
        );
    }

    /// A plugin skill whose package no longer loads must still block deployment instead of
    /// resolving, matching the unavailable state the settings surface shows for it.
    #[test]
    fn rejects_a_plugin_skill_whose_package_is_unavailable() {
        let temp = TempDir::new().unwrap();
        let package_root = temp.path().join("plugins/review-pack/assets/review");
        let pool = test_pool(&temp);
        install_plugin_skill(&pool, &package_root, "review");
        std::fs::remove_dir_all(&package_root).unwrap();
        let storage = FilesystemSkillStorage::new(temp.path().join("skills"));
        let repository = SqliteSkillRepository::with_clock(pool, crate::test_clock::TestClock);

        assert!(matches!(
            resolve_executable_skill_name(&storage, &repository, "review"),
            Err(StartPrerequisitesError::WorkflowSkillNotFound { skill_id })
                if skill_id == "review"
        ));
    }

    /// A workflow node bound to a plugin-imported skill deploys: the receipt freezes the plugin
    /// skill's invocation name and discovery paths like a local skill's.
    #[test]
    fn initialize_workspace_binds_a_plugin_skill_through_its_plugin_package() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let package_root = temp.path().join("plugins/review-pack/assets/review");
        let pool = test_pool(&temp);
        install_plugin_skill(&pool, &package_root, "review");
        let initializer = SkillRoleWorkspaceInitializer::new(skills_root, pool).unwrap();
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"ora-space.codex","modelId":"m"},"skills":[{"skillId":"review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let receipt = initializer.initialize_workspace(&graph, &worktree).unwrap();

        assert!(!worktree.join(".agents").exists());
        assert_eq!(
            receipt,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "a".to_string(),
                    skill_id: "review".to_string(),
                    invocation_name: "review".to_string(),
                    package_paths: vec![
                        StrictRelativePath::parse(".agents/skills/review").unwrap()
                    ],
                }],
            }
        );
    }

    #[test]
    fn initialize_workspace_records_child_loop_skills_without_copying_packages() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("sfmea_review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sfmea_review\ndescription: review\n---\n\nbody\n",
        )
        .unwrap();
        let database_path = temp.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::new(crate::test_clock::TestClock)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool");
        let initializer = SkillRoleWorkspaceInitializer::new(skills_root, pool).unwrap();
        let graph = WorkflowGraph::parse(
            r#"{"schemaVersion":2,"nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"loop","data":{"kind":"loop","loopConfig":{
                    "maxIterations":1,
                    "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["a","output"]}],
                    "until":{"logic":"and","conditions":[{"variableSelector":["a","output"],"operator":"equals","value":"done"}]},
                    "outputs":[{"name":"result","variableSelector":["a","output"]}]
                }}},
                {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
                {"id":"a","data":{"kind":"agent","containerId":"loop","agentConfig":{"executor":{"agentCli":"ora-space.codex","modelId":"m"},"skills":[{"skillId":"sfmea_review","enabled":true}]}}}
            ],"edges":[{"source":"start","target":"loop"},{"source":"entry","target":"a"}]}"#,
        )
        .unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let receipt = initializer.initialize_workspace(&graph, &worktree).unwrap();

        assert!(!worktree.join(".agents").exists());
        assert_eq!(
            receipt,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "a".to_string(),
                    skill_id: "sfmea_review".to_string(),
                    invocation_name: "sfmea-review".to_string(),
                    package_paths: vec![
                        StrictRelativePath::parse(".agents/skills/sfmea_review").unwrap()
                    ],
                }],
            }
        );
    }

    /// The receipt names the path Effect materializes for an underscored catalog name, while the
    /// prompt keeps the Agent-facing hyphenated invocation. Both discovery-root shapes are covered.
    #[test]
    fn initialize_workspace_reports_the_materialized_path_for_an_underscored_skill() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("sfmea_review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sfmea_review\ndescription: review\n---\n\nbody\n",
        )
        .unwrap();
        let pool = test_pool(&temp);
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let codeagent_path =
            materialize_skill_at(&worktree, ".agents/skills", &skill_dir, "sfmea_review");
        let opencode_path =
            materialize_skill_at(&worktree, ".opencode/skills", &skill_dir, "sfmea_review");
        let codeagent_expected = worktree.join(".agents").join("skills").join("sfmea_review");
        let opencode_expected = worktree
            .join(".opencode")
            .join("skills")
            .join("sfmea_review");
        assert_eq!(
            (
                std::fs::canonicalize(&codeagent_path).unwrap(),
                std::fs::canonicalize(&opencode_path).unwrap(),
            ),
            (
                std::fs::canonicalize(&codeagent_expected).unwrap(),
                std::fs::canonicalize(&opencode_expected).unwrap(),
            )
        );

        let initializer = SkillRoleWorkspaceInitializer::with_delivery_provider(
            skills_root,
            pool,
            FixedDeliveryProvider {
                delivery: AgentSkillDelivery::Filesystem {
                    discovery_roots: SkillDiscoveryRoots::new(
                        StrictRelativePath::parse(".agents/skills").unwrap(),
                        vec![StrictRelativePath::parse(".opencode/skills").unwrap()],
                    ),
                },
            },
        );
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"review","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"acme.agent","modelId":"m"},"skills":[{"skillId":"sfmea_review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();

        let receipt = initializer.initialize_workspace(&graph, &worktree).unwrap();

        assert_eq!(
            receipt,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "review".to_string(),
                    skill_id: "sfmea_review".to_string(),
                    invocation_name: "sfmea-review".to_string(),
                    package_paths: vec![
                        StrictRelativePath::parse(".agents/skills/sfmea_review").unwrap(),
                        StrictRelativePath::parse(".opencode/skills/sfmea_review").unwrap(),
                    ],
                }],
            }
        );
        let frozen_paths = receipt.bindings[0].absolute_package_paths(&worktree);
        assert_eq!(
            frozen_paths
                .iter()
                .map(|path| std::fs::canonicalize(path).unwrap())
                .collect::<Vec<_>>(),
            vec![
                std::fs::canonicalize(&codeagent_path).unwrap(),
                std::fs::canonicalize(&opencode_path).unwrap(),
            ]
        );
        assert!(
            frozen_paths
                .iter()
                .all(|path| path.join("SKILL.md").is_file()),
            "every frozen package path must exist: {frozen_paths:?}"
        );

        let prompt = render_required_skills(
            &[RequiredWorkflowSkill {
                invocation_name: receipt.bindings[0].invocation_name.clone(),
                package_paths: frozen_paths.clone(),
            }],
            WorkflowRunLocale::EnUs,
        );
        assert!(prompt.contains("- /sfmea-review"), "{prompt}");
        for path in &frozen_paths {
            assert!(
                prompt.contains(&render_prompt_path(path)),
                "the prompt must name the materialized package path: {prompt}"
            );
        }
        assert!(
            !prompt.contains("sfmea-review/"),
            "the prompt must not substitute the invocation spelling for the package directory: {prompt}"
        );
    }

    /// A mixed-case catalog name resolves to the canonical receipt path and prompt package path
    /// while keeping the Agent-facing invocation spelling.
    #[test]
    fn initialize_workspace_reports_a_mixed_case_skill_at_the_canonical_path() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("SfMeA_Review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: SfMeA_Review\ndescription: review\n---\n\nbody\n",
        )
        .unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let materialized_path =
            materialize_skill_at(&worktree, ".agents/skills", &skill_dir, "SfMeA_Review");
        let initializer =
            SkillRoleWorkspaceInitializer::new(skills_root, test_pool(&temp)).unwrap();
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"review","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"ora-space.codex","modelId":"m"},"skills":[{"skillId":"SfMeA_Review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();

        let receipt = initializer.initialize_workspace(&graph, &worktree).unwrap();

        assert_eq!(
            receipt,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "review".to_string(),
                    skill_id: "SfMeA_Review".to_string(),
                    invocation_name: "sfmea-review".to_string(),
                    package_paths: vec![
                        StrictRelativePath::parse(".agents/skills/sfmea_review").unwrap(),
                    ],
                }],
            }
        );
        let frozen_paths = receipt.bindings[0].absolute_package_paths(&worktree);
        assert_eq!(
            frozen_paths
                .iter()
                .map(|path| std::fs::canonicalize(path).unwrap())
                .collect::<Vec<_>>(),
            vec![std::fs::canonicalize(&materialized_path).unwrap()]
        );
        assert!(frozen_paths[0].join("SKILL.md").is_file());

        let prompt = render_required_skills(
            &[RequiredWorkflowSkill {
                invocation_name: receipt.bindings[0].invocation_name.clone(),
                package_paths: frozen_paths.clone(),
            }],
            WorkflowRunLocale::EnUs,
        );
        assert!(prompt.contains("- /sfmea-review"), "{prompt}");
        assert!(
            prompt.contains(&render_prompt_path(&frozen_paths[0])),
            "the prompt must name the materialized package path: {prompt}"
        );
    }

    /// Two catalog names whose materialized directories differ but whose prompt invocation
    /// normalizes to the same `/name` remain rejected, preserving the previous ambiguity gate.
    #[test]
    fn initialize_workspace_rejects_catalog_names_that_share_a_prompt_invocation() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        for name in ["sfmea_review", "sfmea-review"] {
            let skill_dir = skills_root.join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: review\n---\n\nbody\n"),
            )
            .unwrap();
        }
        let initializer =
            SkillRoleWorkspaceInitializer::new(skills_root, test_pool(&temp)).unwrap();
        assert_ne!(
            materialized_skill_directory(&SkillName::parse("sfmea_review").unwrap()),
            materialized_skill_directory(&SkillName::parse("sfmea-review").unwrap()),
            "the physical directories differ; only the prompt invocation collides"
        );
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"review","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"ora-space.codex","modelId":"m"},"skills":[{"skillId":"sfmea_review","enabled":true},{"skillId":"sfmea-review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let error = initializer
            .initialize_workspace(&graph, &worktree)
            .expect_err("colliding prompt invocations must not deploy");

        assert!(matches!(
            error,
            StartPrerequisitesError::SkillMaterializationError { ref message }
                if message.contains("sfmea_review")
                    && message.contains("sfmea-review")
                    && message.contains("same prompt invocation /sfmea-review")
                    && message.contains("under discovery root .agents/skills")
        ));
    }

    /// The same invocation spelling is unambiguous when each colliding catalog name belongs to a
    /// different Agent discovery root, matching the previous root-plus-invocation conflict key.
    #[test]
    fn initialize_workspace_allows_one_prompt_invocation_per_distinct_discovery_root() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        for name in ["sfmea_review", "sfmea-review"] {
            let skill_dir = skills_root.join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: review\n---\n\nbody\n"),
            )
            .unwrap();
        }
        let deliveries = HashMap::from([
            (
                "acme.codex".to_string(),
                AgentSkillDelivery::Filesystem {
                    discovery_roots: SkillDiscoveryRoots::new(
                        StrictRelativePath::parse(".agents/skills").unwrap(),
                        Vec::new(),
                    ),
                },
            ),
            (
                "acme.opencode".to_string(),
                AgentSkillDelivery::Filesystem {
                    discovery_roots: SkillDiscoveryRoots::new(
                        StrictRelativePath::parse(".opencode/skills").unwrap(),
                        Vec::new(),
                    ),
                },
            ),
        ]);
        let initializer = SkillRoleWorkspaceInitializer::with_delivery_provider(
            skills_root,
            test_pool(&temp),
            AgentKeyedDeliveryProvider { deliveries },
        );
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[
                {"id":"codeagent","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"acme.codex","modelId":"m"},"skills":[{"skillId":"sfmea_review","enabled":true}]}}},
                {"id":"opencode","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"acme.opencode","modelId":"m"},"skills":[{"skillId":"sfmea-review","enabled":true}]}}}
            ],"edges":[]}"#,
        )
        .unwrap();

        let receipt = initializer
            .initialize_workspace(&graph, temp.path())
            .expect("one invocation per distinct discovery root must remain deployable");

        assert_eq!(
            receipt,
            SkillMaterializationReceipt {
                bindings: vec![
                    MaterializedSkillBinding {
                        node_id: "codeagent".to_string(),
                        skill_id: "sfmea_review".to_string(),
                        invocation_name: "sfmea-review".to_string(),
                        package_paths: vec![
                            StrictRelativePath::parse(".agents/skills/sfmea_review").unwrap(),
                        ],
                    },
                    MaterializedSkillBinding {
                        node_id: "opencode".to_string(),
                        skill_id: "sfmea-review".to_string(),
                        invocation_name: "sfmea-review".to_string(),
                        package_paths: vec![
                            StrictRelativePath::parse(".opencode/skills/sfmea-review").unwrap(),
                        ],
                    },
                ],
            }
        );
    }

    /// An injected Agent capability controls the persisted placement receipt without any
    /// prompt-layer directory convention or workflow-owned filesystem writes.
    #[test]
    fn injected_agent_capability_controls_placements_and_the_frozen_receipt() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: review\n---\n",
        )
        .unwrap();
        let first_root = StrictRelativePath::parse(".claude/skills").unwrap();
        let second_root = StrictRelativePath::parse(".vendor/agent-skills").unwrap();
        let initializer = SkillRoleWorkspaceInitializer::with_delivery_provider(
            skills_root,
            test_pool(&temp),
            FixedDeliveryProvider {
                delivery: AgentSkillDelivery::Filesystem {
                    discovery_roots: SkillDiscoveryRoots::new(
                        first_root.clone(),
                        vec![second_root.clone()],
                    ),
                },
            },
        );
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"review-node","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"acme.agent","modelId":"m"},"skills":[{"skillId":"review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let receipt = initializer.initialize_workspace(&graph, &worktree).unwrap();

        let package_paths = vec![
            first_root.append_segment("review"),
            second_root.append_segment("review"),
        ];
        assert_eq!(
            receipt,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "review-node".to_string(),
                    skill_id: "review".to_string(),
                    invocation_name: "review".to_string(),
                    package_paths: package_paths.clone(),
                }],
            }
        );
        assert_eq!(
            package_paths
                .iter()
                .map(|path| path.to_path(&worktree).exists())
                .collect::<Vec<_>>(),
            vec![false, false]
        );
    }

    /// Enabled skills fail deployment when the selected Agent explicitly cannot consume them.
    #[test]
    fn enabled_skills_reject_an_agent_without_delivery_support() {
        let temp = TempDir::new().unwrap();
        let initializer = SkillRoleWorkspaceInitializer::with_delivery_provider(
            temp.path().join("skills"),
            test_pool(&temp),
            FixedDeliveryProvider {
                delivery: AgentSkillDelivery::Unsupported,
            },
        );
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"acme.agent","modelId":"m"},"skills":[{"skillId":"review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();

        assert!(matches!(
            initializer.initialize_workspace(&graph, temp.path()),
            Err(StartPrerequisitesError::AgentSkillDeliveryUnsupported { agent_ref })
                if agent_ref == "acme.agent"
        ));
    }
}

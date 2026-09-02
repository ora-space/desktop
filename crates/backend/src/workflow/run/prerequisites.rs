use ora_application::{
    AgentDefinitionRepository, AgentSkillDelivery, AgentSkillDeliveryProvider,
    FilesystemSkillStorage, MaterializedSkillBinding, NodeType, RepositoryError,
    SkillDiscoveryRoots, SkillMaterializationReceipt, SkillRepository, StartPrerequisitesError,
    WorkflowGraph, WorkflowRunWorkspaceInitializer, has_usable_package,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository, SqliteSkillRepository};
use ora_domain::{AgentDefinitionId, AgentRef, Namespace, SkillId};
use ora_utils::path::StrictRelativePath;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// Validates a run workspace's roles and skill bindings at deploy time.
///
/// Roles and skills are deploy hard-dependencies: every agent's role must resolve in the agents
/// catalog and every enabled skill must exist in the catalog. The Effect subsystem owns physical
/// package materialization; this initializer only freezes the invocation names and Effect-owned
/// discovery paths that execution uses to build the prompt.
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
    for node in graph.nodes() {
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
    roles
}

/// Resolves every node's enabled skills to the Effect-owned paths later consumed by execution.
fn resolve_graph_skill_bindings<DeliveryProvider>(
    storage: &FilesystemSkillStorage,
    skill_repository: &SqliteSkillRepository,
    delivery_provider: &DeliveryProvider,
    graph: &WorkflowGraph,
) -> Result<SkillMaterializationReceipt, StartPrerequisitesError>
where
    DeliveryProvider: AgentSkillDeliveryProvider,
{
    let mut receipt = SkillMaterializationReceipt::default();
    let mut resolved_packages = HashMap::<StrictRelativePath, String>::new();
    for node in graph
        .nodes()
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
                resolve_skill_catalog_name(storage, Some(skill_repository), &skill.skill_id)?;
            let invocation_name = normalize_skill_name(&catalog_name);
            let package_paths = discovery_roots
                .iter()
                .map(|root| root.append_segment(&invocation_name))
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
/// A namespaced id like `cdase:sfmea_review` resolves by the suffix after the colon. When that
/// name is not a catalog directory, `skill_repository` resolves a skill id (the editor stores
/// skill ids as `skillId`) back to the catalog name.
fn resolve_skill_catalog_name(
    storage: &FilesystemSkillStorage,
    skill_repository: Option<&SqliteSkillRepository>,
    skill_id: &str,
) -> Result<String, StartPrerequisitesError> {
    let candidate = skill_id.rsplit(':').next().unwrap_or(skill_id);
    if skill_package_usable(storage, candidate)? {
        return Ok(candidate.to_string());
    }
    if let Some(repository) = skill_repository {
        let Some(skill) = repository
            .find_skill(&SkillId::new(candidate))
            .map_err(StartPrerequisitesError::Repository)?
        else {
            return Err(StartPrerequisitesError::WorkflowSkillNotFound {
                skill_id: skill_id.to_string(),
            });
        };
        if skill_package_usable(storage, &skill.name)? {
            return Ok(skill.name);
        }
    }
    Err(StartPrerequisitesError::WorkflowSkillNotFound {
        skill_id: skill_id.to_string(),
    })
}

/// Returns whether the catalog still has a formal package that Get can load.
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

/// Resolves an enabled skill id to the executable `/name` the agent CLI uses to invoke it: the
/// normalized catalog name, matching the directory materialized by the Effect subsystem.
#[cfg(test)]
fn resolve_executable_skill_name(
    storage: &FilesystemSkillStorage,
    skill_repository: Option<&SqliteSkillRepository>,
    skill_id: &str,
) -> Result<String, StartPrerequisitesError> {
    Ok(normalize_skill_name(&resolve_skill_catalog_name(
        storage,
        skill_repository,
        skill_id,
    )?))
}

/// Normalizes a catalog name for an Agent discovery directory: lowercase, `_` becomes `-`.
fn normalize_skill_name(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
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

    /// Opens an isolated repository pool used by capability-driven binding tests.
    fn test_pool(temp: &TempDir) -> RepositoryPool {
        DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&temp.path().join("ora.sqlite3")),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool")
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
        assert_eq!(
            resolve_executable_skill_name(&storage, None, "cdase:sfmea_review").unwrap(),
            "sfmea-review"
        );
    }

    #[test]
    fn initialize_workspace_records_skill_bindings_without_copying_packages() {
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
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool");
        let initializer = SkillRoleWorkspaceInitializer::new(skills_root, pool).unwrap();
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"ora-space.codex","modelId":"m"},"skills":[{"skillId":"sfmea_review","enabled":true}]}}}],"edges":[]}"#,
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
                        StrictRelativePath::parse(".agents/skills/sfmea-review").unwrap()
                    ],
                }],
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

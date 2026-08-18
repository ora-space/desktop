use ora_application::{
    AgentDefinitionRepository, FilesystemSkillStorage, NodeType, RepositoryError, SkillRepository,
    StartPrerequisitesError, WorkflowGraph, WorkflowRunWorktreeInitializer, has_usable_package,
};
use ora_db::{RepositoryPool, SqliteAgentDefinitionRepository, SqliteSkillRepository};
use ora_domain::{AgentDefinitionId, Namespace, SkillId};
use ora_skill_package::{parse_manifest, rewrite_manifest};
use std::path::{Path, PathBuf};

/// Upper bound for a SKILL.md manifest read during materialization.
const MAX_SKILL_MANIFEST_BYTES: u64 = 1024 * 1024;

/// The cross-tool skill root under the worktree: opencode, Claude Code, and .agents all discover
/// `.agents/skills/<name>/` (the project-shared standard), so materializing there once serves
/// every agent CLI.
const SKILL_DISCOVERY_DIRS: [&str; 1] = [".agents"];

/// Validates and materializes a run worktree's initial state at deploy time.
///
/// Roles and skills are deploy hard-dependencies: every agent's role must resolve in the agents
/// catalog and every enabled skill must exist in the catalog. Enabled skills are copied into
/// `<worktree>/.agents/skills/<normalized>/`, where agent CLIs auto-discover them, so the worktree
/// is complete from the moment the run is created and `start` needs no re-validation.
#[derive(Clone)]
pub struct SkillRoleWorktreeInitializer {
    skills_root: PathBuf,
    pool: RepositoryPool,
}

impl SkillRoleWorktreeInitializer {
    /// Builds an initializer from the skill catalog root and the shared repository pool.
    pub fn new(skills_root: PathBuf, pool: RepositoryPool) -> Self {
        Self { skills_root, pool }
    }
}

impl WorkflowRunWorktreeInitializer for SkillRoleWorktreeInitializer {
    fn initialize_worktree(
        &self,
        graph: &WorkflowGraph,
        worktree_root: &Path,
    ) -> Result<(), StartPrerequisitesError> {
        let (skills, roles) = collect_requirements(graph);

        let agent_repository = SqliteAgentDefinitionRepository::new(self.pool.clone());
        for role_id in &roles {
            if resolve_role(&agent_repository, role_id)?.is_none() {
                return Err(StartPrerequisitesError::WorkflowRoleNotFound {
                    role_id: role_id.clone(),
                });
            }
        }

        if !skills.is_empty() {
            let storage = FilesystemSkillStorage::new(self.skills_root.clone());
            let skill_repository = SqliteSkillRepository::new(self.pool.clone());
            for skill_id in &skills {
                materialize_skill(&storage, Some(&skill_repository), worktree_root, skill_id)?;
            }
        }
        Ok(())
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

/// Collects the distinct enabled skill ids and role ids declared across all agent nodes.
fn collect_requirements(graph: &WorkflowGraph) -> (Vec<String>, Vec<String>) {
    let mut skills = Vec::new();
    let mut roles = Vec::new();
    for node in graph.nodes() {
        if node.node_type != NodeType::Agent {
            continue;
        }
        let Some(config) = &node.agent_config else {
            continue;
        };
        for skill in &config.skills {
            if skill.enabled && !skills.contains(&skill.skill_id) {
                skills.push(skill.skill_id.clone());
            }
        }
        if let Some(role_id) = &config.role_id
            && !role_id.trim().is_empty()
            && !roles.contains(role_id)
        {
            roles.push(role_id.clone());
        }
    }
    (skills, roles)
}

/// Resolves one enabled skill against the catalog and copies it into the worktree.
///
/// The catalog name comes from `resolve_skill_catalog_name`; the worktree directory uses its
/// normalized form so agent CLIs discover the package as `/name`.
fn materialize_skill(
    storage: &FilesystemSkillStorage,
    skill_repository: Option<&SqliteSkillRepository>,
    worktree_root: &Path,
    skill_id: &str,
) -> Result<(), StartPrerequisitesError> {
    let catalog_name = resolve_skill_catalog_name(storage, skill_repository, skill_id)?;
    let dir_name = normalize_skill_name(&catalog_name);
    for discovery_dir in SKILL_DISCOVERY_DIRS {
        let target = worktree_root
            .join(discovery_dir)
            .join("skills")
            .join(&dir_name);
        storage
            .copy_package_to(&catalog_name, &target)
            .map_err(|error| StartPrerequisitesError::SkillMaterializationError {
                message: error.to_string(),
            })?;
        rewrite_manifest_name(&target, &dir_name)
            .map_err(|message| StartPrerequisitesError::SkillMaterializationError { message })?;
    }
    Ok(())
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
/// normalized catalog name, matching the directory it was materialized into.
pub(super) fn resolve_executable_skill_name(
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

/// Normalizes a catalog name for the `.agents/skills/` directory: lowercase, `_` becomes `-`.
fn normalize_skill_name(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

/// Rewrites the copied `SKILL.md` frontmatter `name` when it differs from the target directory.
fn rewrite_manifest_name(target: &Path, dir_name: &str) -> Result<(), String> {
    let manifest_path = target.join("SKILL.md");
    let bytes = std::fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let manifest = parse_manifest(&bytes, MAX_SKILL_MANIFEST_BYTES)
        .map_err(|error| format!("invalid SKILL.md in {}: {error}", manifest_path.display()))?;
    if manifest.name == dir_name {
        return Ok(());
    }
    let rewritten = rewrite_manifest(&bytes, dir_name, &manifest.description)
        .map_err(|error| format!("failed to rewrite SKILL.md name: {error}"))?;
    std::fs::write(&manifest_path, rewritten).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

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
    fn initialize_worktree_materializes_skills_into_the_given_worktree() {
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
        let initializer = SkillRoleWorktreeInitializer::new(skills_root, pool);
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"skills":[{"skillId":"sfmea_review","enabled":true}]}}}],"edges":[]}"#,
        )
        .unwrap();
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        initializer.initialize_worktree(&graph, &worktree).unwrap();

        assert!(
            worktree
                .join(".agents")
                .join("skills")
                .join("sfmea-review")
                .join("SKILL.md")
                .is_file(),
            "enabled skill is materialized into the worktree's initial state"
        );
    }

    #[test]
    fn materialize_skill_copies_the_package_and_rewrites_the_manifest() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("sfmea_review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sfmea_review\ndescription: review\n---\n\nbody\n",
        )
        .unwrap();
        std::fs::write(skill_dir.join("notes.txt"), "payload").unwrap();
        let storage = FilesystemSkillStorage::new(skills_root);
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        materialize_skill(&storage, None, &worktree, "cdase:sfmea_review").unwrap();

        // The package lands under every CLI discovery root so the agent in use finds it.
        for discovery_dir in SKILL_DISCOVERY_DIRS {
            let target = worktree
                .join(discovery_dir)
                .join("skills")
                .join("sfmea-review");
            assert!(
                target.join("notes.txt").exists(),
                "missing under {discovery_dir}"
            );
            let manifest = parse_manifest(
                &std::fs::read(target.join("SKILL.md")).unwrap(),
                MAX_SKILL_MANIFEST_BYTES,
            )
            .unwrap();
            assert_eq!(manifest.name, "sfmea-review");
            assert_eq!(manifest.description, "review");
        }
    }

    #[test]
    fn materialize_skill_reports_an_unreadable_manifest_as_missing() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("broken");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: [unclosed").unwrap();
        let storage = FilesystemSkillStorage::new(skills_root);
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let error = materialize_skill(&storage, None, &worktree, "broken").unwrap_err();
        assert!(matches!(
            error,
            StartPrerequisitesError::WorkflowSkillNotFound { skill_id }
                if skill_id == "broken"
        ));
    }

    #[test]
    fn materialize_skill_reports_a_missing_skill() {
        let temp = TempDir::new().unwrap();
        let storage = FilesystemSkillStorage::new(temp.path().join("skills"));
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let error = materialize_skill(&storage, None, &worktree, "missing_skill").unwrap_err();
        assert!(matches!(
            error,
            StartPrerequisitesError::WorkflowSkillNotFound { skill_id }
                if skill_id == "missing_skill"
        ));
    }

    #[test]
    fn materialize_skill_is_idempotent_and_overwrites() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let skill_dir = skills_root.join("explore");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: explore\ndescription: explore\n---\n\nbody\n",
        )
        .unwrap();
        let storage = FilesystemSkillStorage::new(skills_root);
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        materialize_skill(&storage, None, &worktree, "explore").unwrap();
        materialize_skill(&storage, None, &worktree, "explore").unwrap();
        assert!(
            worktree
                .join(".agents")
                .join("skills")
                .join("explore")
                .join("SKILL.md")
                .exists()
        );
    }
}

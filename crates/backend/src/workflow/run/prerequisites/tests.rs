use super::*;
use ora_application::{SkillMaterializationReceipt, StartPrerequisitesError, WorkflowGraph};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, SqliteSkillRepository,
    default_migration_catalog,
};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId};
use pretty_assertions::assert_eq;
use std::path::Path;
use tempfile::TempDir;

fn test_pool(temp: &TempDir) -> RepositoryPool {
    DatabaseBootstrapper::new(crate::test_clock::TestClock)
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("migration catalog"),
        )
        .expect("bootstrap test database")
}

fn install_local_skill(pool: &RepositoryPool, skills_root: &Path, id: &str, name: &str) {
    let package_dir = skills_root.join(name);
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(
        package_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: local skill\n---\n"),
    )
    .unwrap();
    SqliteSkillRepository::new(pool.clone())
        .create_skill(
            Skill::new(
                SkillId::new(id),
                Namespace::local(),
                name,
                "local skill",
                AuditFields::new(1, 1, /*is_deleted*/ false),
            )
            .unwrap(),
        )
        .unwrap();
}

fn install_plugin_skill(pool: &RepositoryPool, package_root: &Path, name: &str) {
    std::fs::create_dir_all(package_root).unwrap();
    std::fs::write(
        package_root.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: plugin skill\n---\n"),
    )
    .unwrap();
    SqliteSkillRepository::new(pool.clone())
        .replace_plugin_skills(
            &PluginId::new("official", "review-pack").unwrap(),
            "1.0.0",
            &[PluginSkillProjection {
                name: name.to_string(),
                description: "plugin skill".to_string(),
                package_root: package_root.to_path_buf(),
            }],
            1,
        )
        .unwrap();
}

#[test]
fn normalizes_skill_names_to_lowercase_dashes() {
    assert_eq!(normalize_skill_name("sfmea_review"), "sfmea-review");
    assert_eq!(normalize_skill_name("OpenSpec_Explore"), "openspec-explore");
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/workflow-skill-resolution.md`:
/// freezes `{ node_id, skill_id, invocation_name }` against catalog rows without writing into the
/// run worktree, and resolves both exact catalog UUIDs and namespaced plugin IDs.
#[test]
fn freezes_catalog_bindings_for_local_and_plugin_skills_without_writing_workspace() {
    let temp = TempDir::new().unwrap();
    let pool = test_pool(&temp);
    let skills_root = temp.path().join("atoms").join("skills");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();

    install_local_skill(&pool, &skills_root, "uuid-local-1", "SfMeA_Review");
    let plugin_pkg = temp
        .path()
        .join("plugins/installed/official/review-pack/1.0.0/skills/audit");
    install_plugin_skill(&pool, &plugin_pkg, "audit_code");

    let graph = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"agent","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run",
                    "skills":[
                        {"skillId":"uuid-local-1","enabled":true},
                        {"skillId":"plugin:official/review-pack:audit_code","enabled":true}
                    ]
                }}},
                {"id":"out","data":{"kind":"output"}}
            ],
            "edges":[
                {"source":"start","target":"agent"},
                {"source":"agent","target":"out"}
            ]
        }"#,
    )
    .unwrap();

    let receipt = SkillRoleWorkspaceInitializer::new(skills_root.clone(), pool.clone())
        .initialize_workspace(&graph, &worktree)
        .unwrap();

    assert_eq!(
        receipt,
        SkillMaterializationReceipt {
            bindings: vec![
                MaterializedSkillBinding {
                    node_id: "agent".to_string(),
                    skill_id: "uuid-local-1".to_string(),
                    invocation_name: "sfmea-review".to_string(),
                },
                MaterializedSkillBinding {
                    node_id: "agent".to_string(),
                    skill_id: "plugin:official/review-pack:audit_code".to_string(),
                    invocation_name: "audit-code".to_string(),
                },
            ],
        }
    );
    assert!(
        !worktree.join(".claude").exists() && !worktree.join(".agents").exists(),
        "initializer must not materialize skill files into the workspace"
    );

    let repo = SqliteSkillRepository::new(pool);
    assert_eq!(
        resolve_live_skill_entry(&skills_root, &repo, "uuid-local-1").unwrap(),
        Some(ResolvedCatalogSkill {
            invocation_name: "sfmea-review".to_string(),
            skill_md_path: skills_root.join("SfMeA_Review").join("SKILL.md"),
        })
    );
    assert_eq!(
        resolve_live_skill_entry(
            &skills_root,
            &repo,
            "plugin:official/review-pack:audit_code"
        )
        .unwrap(),
        Some(ResolvedCatalogSkill {
            invocation_name: "audit-code".to_string(),
            skill_md_path: plugin_pkg.join("SKILL.md"),
        })
    );
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/workflow-skill-resolution.md`:
/// soft-deleted or broken catalog skills are rejected with `WorkflowSkillNotFound` even if a
/// same-named directory or another skill exists.
#[test]
fn rejects_missing_deleted_or_unusable_catalog_skills() {
    let temp = TempDir::new().unwrap();
    let pool = test_pool(&temp);
    let skills_root = temp.path().join("atoms").join("skills");
    install_local_skill(&pool, &skills_root, "uuid-review", "review");

    // Remove SKILL.md so catalog entry exists but package is unusable.
    std::fs::remove_file(skills_root.join("review").join("SKILL.md")).unwrap();

    let graph = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"agent","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run",
                    "skills":[{"skillId":"uuid-review","enabled":true}]
                }}},
                {"id":"out","data":{"kind":"output"}}
            ],
            "edges":[
                {"source":"start","target":"agent"},
                {"source":"agent","target":"out"}
            ]
        }"#,
    )
    .unwrap();

    let initializer = SkillRoleWorkspaceInitializer::new(skills_root.clone(), pool.clone());
    assert!(matches!(
        initializer.resolve_graph(&graph),
        Err(StartPrerequisitesError::WorkflowSkillNotFound { skill_id, .. })
            if skill_id == "uuid-review"
    ));

    // Restoring SKILL.md succeeds; soft-deleting the row rejects again.
    std::fs::write(
        skills_root.join("review").join("SKILL.md"),
        "---\nname: review\ndescription: restored\n---\n",
    )
    .unwrap();
    assert!(initializer.resolve_graph(&graph).is_ok());

    let repo = SqliteSkillRepository::new(pool);
    assert!(
        repo.soft_delete_skill(&SkillId::new("uuid-review"), 99)
            .unwrap()
    );
    assert!(matches!(
        initializer.resolve_graph(&graph),
        Err(StartPrerequisitesError::WorkflowSkillNotFound { skill_id, .. })
            if skill_id == "uuid-review"
    ));
}

#[test]
fn initialize_workspace_records_child_loop_skills_without_copying_packages() {
    let temp = TempDir::new().unwrap();
    let pool = test_pool(&temp);
    let skills_root = temp.path().join("atoms").join("skills");
    install_local_skill(&pool, &skills_root, "review", "review");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();

    let graph = WorkflowGraph::parse(
        r#"{"schemaVersion":2,"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"loop","data":{"kind":"loop","loopConfig":{
                "maxIterations":1,
                "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["inner","output"]}],
                "until":{"logic":"and","conditions":[{"variableSelector":["inner","output"],"operator":"equals","value":"done"}]},
                "outputs":[{"name":"result","variableSelector":["inner","output"]}]
            }}},
            {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
            {"id":"inner","data":{"kind":"agent","containerId":"loop","agentConfig":{"executor":{"agentCli":"opencode","modelId":"m"},"skills":[{"skillId":"review","enabled":true}]}}}
        ],"edges":[{"source":"start","target":"loop"},{"source":"entry","target":"inner"}]}"#,
    )
    .unwrap();

    let receipt = SkillRoleWorkspaceInitializer::new(skills_root, pool)
        .initialize_workspace(&graph, &worktree)
        .unwrap();

    assert_eq!(
        receipt,
        SkillMaterializationReceipt {
            bindings: vec![MaterializedSkillBinding {
                node_id: "inner".to_string(),
                skill_id: "review".to_string(),
                invocation_name: "review".to_string(),
            }],
        }
    );
}

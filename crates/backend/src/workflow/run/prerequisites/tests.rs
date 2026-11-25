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
                    catalog_skill_id: Some("uuid-local-1".to_string()),
                    invocation_name: "sfmea-review".to_string(),
                },
                MaterializedSkillBinding {
                    node_id: "agent".to_string(),
                    skill_id: "plugin:official/review-pack:audit_code".to_string(),
                    catalog_skill_id: Some("plugin:official/review-pack:audit_code".to_string()),
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
        resolve_exact_catalog_skill_entry(&skills_root, &repo, "uuid-local-1").unwrap(),
        Some(ResolvedCatalogSkill {
            catalog_skill_id: "uuid-local-1".to_string(),
            catalog_name: "SfMeA_Review".to_string(),
            invocation_name: "sfmea-review".to_string(),
            skill_md_path: skills_root.join("SfMeA_Review").join("SKILL.md"),
        })
    );
    assert_eq!(
        resolve_exact_catalog_skill_entry(
            &skills_root,
            &repo,
            "plugin:official/review-pack:audit_code"
        )
        .unwrap(),
        Some(ResolvedCatalogSkill {
            catalog_skill_id: "plugin:official/review-pack:audit_code".to_string(),
            catalog_name: "audit_code".to_string(),
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
                catalog_skill_id: Some("review".to_string()),
                invocation_name: "review".to_string(),
            }],
        }
    );
}

/// Covers D8 item 3: an uninstalled plugin skill or soft-deleted exact ID must never silently
/// fall back to a same-named active skill during graph resolution or frozen-receipt validation.
#[test]
fn uninstalled_plugin_or_deleted_id_never_falls_back_to_same_named_skill() {
    let temp = TempDir::new().unwrap();
    let pool = test_pool(&temp);
    let skills_root = temp.path().join("atoms").join("skills");
    let plugin_pkg = temp
        .path()
        .join("plugins/installed/official/review-pack/1.0.0/assets/review");
    install_plugin_skill(&pool, &plugin_pkg, "review");

    // Editor-authored bare name freezes the exact plugin catalog ID.
    let bare_graph = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"agent","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run",
                    "skills":[{"skillId":"review","enabled":true}]
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
    let receipt = initializer.resolve_graph(&bare_graph).unwrap();
    assert_eq!(
        receipt.bindings[0].catalog_skill_id.as_deref(),
        Some("plugin:official/review-pack:review")
    );

    // Uninstall the plugin and install a same-named local skill.
    let repo = SqliteSkillRepository::new(pool.clone());
    repo.remove_plugin_skills(&PluginId::new("official", "review-pack").unwrap(), 2)
        .unwrap();
    install_local_skill(&pool, &skills_root, "uuid-local-review", "review");

    // Re-validating the frozen receipt must reject instead of switching to the local skill.
    assert!(matches!(
        initializer.validate_frozen_graph(&bare_graph, &receipt),
        Err(StartPrerequisitesError::WorkflowSkillNotFound { skill_id, .. })
            if skill_id == "review"
    ));

    // Direct plugin ID reference must also reject rather than stripping after `:` to match local.
    let plugin_id_graph = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"agent","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run",
                    "skills":[{"skillId":"plugin:official/review-pack:review","enabled":true}]
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
    assert!(matches!(
        initializer.resolve_graph(&plugin_id_graph),
        Err(StartPrerequisitesError::WorkflowSkillNotFound { skill_id, .. })
            if skill_id == "plugin:official/review-pack:review"
    ));

    // A never-installed plugin ID must likewise never strip to match a same-named local skill.
    assert_eq!(
        resolve_graph_ref_skill_entry(&skills_root, &repo, "plugin:official/other-pack:review")
            .unwrap(),
        None
    );

    // Soft-deleting a local skill whose ID equals its name must not fall back to another skill.
    let other_plugin_pkg = temp
        .path()
        .join("plugins/installed/official/review-pack/1.0.0/assets/other");
    install_plugin_skill(&pool, &other_plugin_pkg, "review");
    repo.soft_delete_skill(&SkillId::new("uuid-local-review"), 3)
        .unwrap();
    assert_eq!(
        resolve_graph_ref_skill_entry(&skills_root, &repo, "uuid-local-review").unwrap(),
        None
    );
}

/// Covers D8 item 6: two distinct skills that normalize to the same `/invocation` within one run
/// are rejected at precheck, while reusing the same skill across nodes is permitted.
#[test]
fn rejects_conflicting_invocation_names_across_distinct_skills_and_allows_reuse() {
    let temp = TempDir::new().unwrap();
    let pool = test_pool(&temp);
    let skills_root = temp.path().join("atoms").join("skills");
    install_local_skill(&pool, &skills_root, "uuid-underscore", "qa_review");
    install_local_skill(&pool, &skills_root, "uuid-hyphen", "qa-review");

    let initializer = SkillRoleWorkspaceInitializer::new(skills_root.clone(), pool.clone());

    let conflicting = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"a","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run a",
                    "skills":[{"skillId":"uuid-underscore","enabled":true}]
                }}},
                {"id":"b","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run b",
                    "skills":[{"skillId":"uuid-hyphen","enabled":true}]
                }}},
                {"id":"out","data":{"kind":"output"}}
            ],
            "edges":[
                {"source":"start","target":"a"},
                {"source":"a","target":"b"},
                {"source":"b","target":"out"}
            ]
        }"#,
    )
    .unwrap();
    let error = initializer.resolve_graph(&conflicting).unwrap_err();
    assert!(
        matches!(
            &error,
            StartPrerequisitesError::SkillMaterializationError { message }
                if message.contains("/qa-review")
        ),
        "expected invocation collision error, got: {error:?}"
    );

    // Reusing the exact same skill across two nodes is allowed.
    let reused = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"a","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run a",
                    "skills":[{"skillId":"uuid-underscore","enabled":true}]
                }}},
                {"id":"b","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run b",
                    "skills":[{"skillId":"qa_review","enabled":true}]
                }}},
                {"id":"out","data":{"kind":"output"}}
            ],
            "edges":[
                {"source":"start","target":"a"},
                {"source":"a","target":"b"},
                {"source":"b","target":"out"}
            ]
        }"#,
    )
    .unwrap();
    let receipt = initializer.resolve_graph(&reused).unwrap();
    assert_eq!(receipt.bindings.len(), 2);
    assert_eq!(
        receipt.bindings[0].catalog_skill_id.as_deref(),
        Some("uuid-underscore")
    );
    assert_eq!(
        receipt.bindings[1].catalog_skill_id.as_deref(),
        Some("uuid-underscore")
    );
}

/// Covers owner Decision A: when a bare skill name matches both a local and a plugin skill (or
/// multiple active catalog entries), graph resolution rejects with the skill-dependency error
/// naming the matching candidates, while explicit catalog IDs still resolve unambiguously.
#[test]
fn rejects_ambiguous_bare_skill_name_while_allowing_exact_catalog_ids() {
    let temp = TempDir::new().unwrap();
    let pool = test_pool(&temp);
    let skills_root = temp.path().join("atoms").join("skills");
    install_local_skill(&pool, &skills_root, "uuid-local-review", "review");
    let plugin_pkg = temp
        .path()
        .join("plugins/installed/official/review-pack/1.0.0/assets/review");
    install_plugin_skill(&pool, &plugin_pkg, "review");

    let initializer = SkillRoleWorkspaceInitializer::new(skills_root.clone(), pool.clone());

    let bare_graph = WorkflowGraph::parse(
        r#"{
            "nodes":[
                {"id":"start","data":{"kind":"start"}},
                {"id":"agent","data":{"kind":"agent","title":"Code review","agentConfig":{
                    "executor":{"agentCli":"opencode","modelId":"m"},
                    "prompt":"run",
                    "skills":[{"skillId":"review","enabled":true}]
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
    let error = initializer.resolve_graph(&bare_graph).unwrap_err();
    assert!(
        matches!(
            &error,
            StartPrerequisitesError::SkillMaterializationError { message }
                if message.contains("Code review")
                    && message.contains("review")
                    && message.contains("uuid-local-review")
                    && message.contains("plugin:official/review-pack:review")
                    && message.contains("specify a concrete skill")
        ),
        "expected ambiguous skill dependency error, got: {error:?}"
    );

    // Selecting either concrete catalog ID succeeds unambiguously even while both exist.
    for exact_id in ["uuid-local-review", "plugin:official/review-pack:review"] {
        let exact_graph = WorkflowGraph::parse(&format!(
            r#"{{
                "nodes":[
                    {{"id":"start","data":{{"kind":"start"}}}},
                    {{"id":"agent","data":{{"kind":"agent","agentConfig":{{
                        "executor":{{"agentCli":"opencode","modelId":"m"}},
                        "prompt":"run",
                        "skills":[{{"skillId":"{exact_id}","enabled":true}}]
                    }}}}}},
                    {{"id":"out","data":{{"kind":"output"}}}}
                ],
                "edges":[
                    {{"source":"start","target":"agent"}},
                    {{"source":"agent","target":"out"}}
                ]
            }}"#
        ))
        .unwrap();
        let receipt = initializer
            .resolve_graph(&exact_graph)
            .expect("exact catalog ID must resolve despite same-named sibling");
        assert_eq!(
            receipt.bindings[0].catalog_skill_id.as_deref(),
            Some(exact_id)
        );
    }
}

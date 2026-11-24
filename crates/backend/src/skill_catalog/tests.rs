use super::DesktopSkillCatalog;
use ora_agent_runtime::{SkillCatalogEntry, SkillCatalogSource};
use ora_application::SkillRepository as _;
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, RepositoryPool,
    SqliteSkillRepository, default_migration_catalog,
};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId, WorkspaceId};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn test_pool(root: &Path) -> RepositoryPool {
    DatabaseBootstrapper::new(crate::test_clock::TestClock)
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("ora.sqlite3")),
            &default_migration_catalog().expect("migration catalog"),
        )
        .expect("bootstrap pool")
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/catalog-delivery.md` &
/// `session-setup-and-hosts.md`: local skills resolve to `<app_data>/atoms/skills/<name>`,
/// plugin skills resolve to their installed `package_root`, invalid/incomplete packages are
/// excluded, and subsequent catalog queries reflect live additions/removals for delta delivery.
#[test]
fn resolves_usable_local_and_plugin_skill_paths_and_excludes_broken_packages() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        let home_dir = temp.path().join("home");
        let skills_root = app_data.join("atoms").join("skills");
        let plugins_installed = ora_plugin_manager::installed_root(&home_dir);

        let local_valid_dir = skills_root.join("local-valid");
        fs::create_dir_all(&local_valid_dir).expect("create valid local skill dir");
        fs::write(
            local_valid_dir.join("SKILL.md"),
            "---\nname: local-valid\ndescription: Valid local skill\n---\nInstructions",
        )
        .expect("write local-valid SKILL.md");

        // Broken local skill (missing SKILL.md) stays in DB as row but must be filtered out.
        fs::create_dir_all(skills_root.join("local-broken"))
            .expect("create broken local skill dir");

        let plugin_valid_dir = plugins_installed.join("official/review/1.0.0/assets/skills/audit");
        fs::create_dir_all(&plugin_valid_dir).expect("create valid plugin skill dir");
        fs::write(
            plugin_valid_dir.join("SKILL.md"),
            "---\nname: audit\ndescription: Audits changes\n---\nPlugin instructions",
        )
        .expect("write plugin SKILL.md");

        let pool = test_pool(&app_data);
        let repo = SqliteSkillRepository::new(pool.clone());
        repo.create_skill(
            Skill::new(
                SkillId::new("skill-local-valid"),
                Namespace::local(),
                "local-valid",
                "Valid local skill",
                AuditFields::new(10, 10, /*is_deleted*/ false),
            )
            .expect("valid skill"),
        )
        .expect("insert local-valid");
        repo.create_skill(
            Skill::new(
                SkillId::new("skill-local-broken"),
                Namespace::local(),
                "local-broken",
                "Broken local skill",
                AuditFields::new(11, 11, /*is_deleted*/ false),
            )
            .expect("broken skill"),
        )
        .expect("insert local-broken");

        let plugin_id = PluginId::new("official", "review").expect("plugin id");
        repo.replace_plugin_skills(
            &plugin_id,
            "1.0.0",
            &[
                PluginSkillProjection {
                    name: "audit".to_string(),
                    description: "Audits changes".to_string(),
                    package_root: plugin_valid_dir.clone(),
                },
                PluginSkillProjection {
                    name: "missing-manifest".to_string(),
                    description: "Broken plugin skill".to_string(),
                    package_root: plugins_installed.join("official/review/1.0.0/assets/missing"),
                },
            ],
            12,
        )
        .expect("insert plugin skills");

        let catalog = DesktopSkillCatalog::new(pool, skills_root.clone(), &home_dir);
        assert_eq!(
            catalog.extra_read_roots(),
            vec![skills_root.clone(), plugins_installed]
        );

        let entries = catalog.skill_catalog(&WorkspaceId::new("ws-1"));
        assert_eq!(
            entries,
            vec![
                SkillCatalogEntry {
                    name: "local-valid".to_string(),
                    description: "Valid local skill".to_string(),
                    skill_md_path: local_valid_dir.join("SKILL.md"),
                    package_dir: local_valid_dir.clone(),
                },
                SkillCatalogEntry {
                    name: "audit".to_string(),
                    description: "Audits changes".to_string(),
                    skill_md_path: plugin_valid_dir.join("SKILL.md"),
                    package_dir: plugin_valid_dir,
                },
            ]
        );

        // Soft-delete local-valid and remove plugin skill -> catalog shrinks cleanly for next delta.
        assert!(
            repo.soft_delete_skill(&SkillId::new("skill-local-valid"), 20)
                .expect("soft delete local-valid")
        );
        repo.remove_plugin_skills(&plugin_id, 21)
            .expect("remove plugin skills");

        assert_eq!(catalog.skill_catalog(&WorkspaceId::new("ws-1")), vec![]);
    });
}

#[cfg(windows)]
#[test]
fn strips_windows_verbatim_prefix_from_persisted_plugin_package_roots() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        let home_dir = temp.path().join("home");
        let skills_root = app_data.join("atoms").join("skills");
        let plugins_installed = ora_plugin_manager::installed_root(&home_dir);

        let plugin_dir = plugins_installed.join("official/review/1.0.0/assets/audit");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("SKILL.md"),
            "---\nname: audit\ndescription: Audits changes\n---\nPlugin instructions",
        )
        .expect("write plugin SKILL.md");

        let canonical_verbatim =
            std::fs::canonicalize(&plugin_dir).expect("canonicalize plugin dir");
        let expected_plain = ora_utils::path::strip_verbatim_prefix(&canonical_verbatim);
        fs::create_dir_all(&app_data).expect("create app data dir");
        let pool = test_pool(&app_data);
        let repo = SqliteSkillRepository::new(pool.clone());
        let plugin_id = PluginId::new("official", "review").expect("plugin id");
        repo.replace_plugin_skills(
            &plugin_id,
            "1.0.0",
            &[PluginSkillProjection {
                name: "audit".to_string(),
                description: "Audits changes".to_string(),
                package_root: canonical_verbatim,
            }],
            1,
        )
        .expect("insert plugin skill");

        let catalog = DesktopSkillCatalog::new(pool, skills_root, &home_dir);
        let entries = catalog.skill_catalog(&WorkspaceId::new("ws-1"));
        assert_eq!(
            entries,
            vec![SkillCatalogEntry {
                name: "audit".to_string(),
                description: "Audits changes".to_string(),
                skill_md_path: expected_plain.join("SKILL.md"),
                package_dir: expected_plain,
            }]
        );
    });
}

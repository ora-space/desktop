use super::DesktopSkillCatalog;
use ora_agent_runtime::{SkillCatalogEntry, SkillCatalogSnapshot, SkillCatalogSource};
use ora_application::SkillRepository as _;
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, RepositoryPool,
    SqliteSkillRepository, default_migration_catalog,
};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId, WorkspaceId};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::collections::BTreeSet;
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

        let snapshot = catalog
            .skill_catalog(&WorkspaceId::new("ws-1"))
            .expect("read usable skill catalog");
        assert_eq!(
            snapshot,
            SkillCatalogSnapshot {
                entries: vec![
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
                ],
                unreadable_paths: BTreeSet::new(),
            }
        );

        // Soft-delete local-valid and remove plugin skill -> catalog shrinks cleanly for next delta.
        assert!(
            repo.soft_delete_skill(&SkillId::new("skill-local-valid"), 20)
                .expect("soft delete local-valid")
        );
        repo.remove_plugin_skills(&plugin_id, 21)
            .expect("remove plugin skills");

        assert_eq!(
            catalog
                .skill_catalog(&WorkspaceId::new("ws-1"))
                .expect("read emptied skill catalog"),
            SkillCatalogSnapshot::default()
        );
    });
}

#[test]
fn returns_error_when_skill_repository_query_fails() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        fs::create_dir_all(&app_data).expect("create app data dir");
        let home_dir = temp.path().join("home");
        let skills_root = app_data.join("atoms").join("skills");
        let db_path = app_data.join("ora.sqlite3");
        let pool = test_pool(&app_data);

        // Drop the `skills` table directly to simulate a database read error.
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
        conn.execute("DROP TABLE skills", [])
            .expect("drop skills table");
        drop(conn);

        let catalog = DesktopSkillCatalog::new(pool, skills_root, &home_dir);
        let error = catalog
            .skill_catalog(&WorkspaceId::new("ws-1"))
            .expect_err("database read failure must return an error rather than an empty catalog");
        assert!(
            error.to_string().contains("failed to query skill catalog"),
            "unexpected error: {error}"
        );
    });
}

#[test]
fn marks_unreadable_skill_manifest_unknown_without_hiding_readable_skills() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        let home_dir = temp.path().join("home");
        let skills_root = app_data.join("atoms").join("skills");

        let readable_dir = skills_root.join("readable-skill");
        fs::create_dir_all(&readable_dir).expect("create readable skill dir");
        fs::write(
            readable_dir.join("SKILL.md"),
            "---\nname: readable-skill\ndescription: Readable\n---\nBody",
        )
        .expect("write readable SKILL.md");

        let local_dir = skills_root.join("locked-skill");
        fs::create_dir_all(&local_dir).expect("create locked skill dir");
        let manifest_path = local_dir.join("SKILL.md");
        fs::write(
            &manifest_path,
            "---\nname: locked-skill\ndescription: Locked\n---\nBody",
        )
        .expect("write SKILL.md");

        let pool = test_pool(&app_data);
        let repo = SqliteSkillRepository::new(pool.clone());
        repo.create_skill(
            Skill::new(
                SkillId::new("skill-readable"),
                Namespace::local(),
                "readable-skill",
                "Readable",
                AuditFields::new(9, 9, /*is_deleted*/ false),
            )
            .expect("readable skill"),
        )
        .expect("insert readable skill");
        repo.create_skill(
            Skill::new(
                SkillId::new("skill-locked"),
                Namespace::local(),
                "locked-skill",
                "Locked",
                AuditFields::new(10, 10, /*is_deleted*/ false),
            )
            .expect("locked skill"),
        )
        .expect("insert locked skill");

        let catalog = DesktopSkillCatalog::new(pool, skills_root, &home_dir);

        #[cfg(windows)]
        let _lock = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&manifest_path)
                .expect("exclusively lock SKILL.md on Windows")
        };

        #[cfg(unix)]
        let _restore_perms = {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o000))
                .expect("revoke read permissions on SKILL.md");
            struct RestorePerms(std::path::PathBuf);
            impl Drop for RestorePerms {
                fn drop(&mut self) {
                    let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o644));
                }
            }
            // Root ignores 0o000 permissions on Unix; only assert when fs::read actually fails.
            if fs::read(&manifest_path).is_ok() {
                let _ = fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o644));
                return;
            }
            RestorePerms(manifest_path.clone())
        };

        let snapshot = catalog
            .skill_catalog(&WorkspaceId::new("ws-1"))
            .expect("per-entry manifest I/O error must not fail the whole catalog");
        assert_eq!(
            snapshot,
            SkillCatalogSnapshot {
                entries: vec![SkillCatalogEntry {
                    name: "readable-skill".to_string(),
                    description: "Readable".to_string(),
                    skill_md_path: readable_dir.join("SKILL.md"),
                    package_dir: readable_dir,
                }],
                unreadable_paths: BTreeSet::from([manifest_path]),
            }
        );
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
        let snapshot = catalog
            .skill_catalog(&WorkspaceId::new("ws-1"))
            .expect("read plugin skill catalog");
        assert_eq!(
            snapshot,
            SkillCatalogSnapshot {
                entries: vec![SkillCatalogEntry {
                    name: "audit".to_string(),
                    description: "Audits changes".to_string(),
                    skill_md_path: expected_plain.join("SKILL.md"),
                    package_dir: expected_plain,
                }],
                unreadable_paths: BTreeSet::new(),
            }
        );
    });
}

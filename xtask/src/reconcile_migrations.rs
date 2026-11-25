use std::path::Path;

use ora_db::{
    DatabaseLocation, backup_database_before_0025, cleanup_legacy_managed_skills,
    default_migration_catalog, reconcile_migration_history,
};

/// Reconciles the development database against current migration SQL before Desktop starts.
///
/// Backs up an existing pre-`0025` database and runs the D9 legacy Effect-managed skill directory
/// cleanup before migration `0025` drops the Effect ledger so development startup
/// (`task run:desktop`) follows the same safety steps as packaged application bootstrap.
pub fn run_reconcile_migrations(data_directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_directory)?;
    let database_path = data_directory.join("ora.sqlite3");
    backup_database_before_0025(&database_path)?;
    cleanup_legacy_managed_skills(&database_path);
    let catalog = default_migration_catalog()?;
    reconcile_migration_history(&DatabaseLocation::path(database_path), &catalog)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run_reconcile_migrations;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, MigrationCatalog, default_migration_catalog,
    };
    use ora_utils::directory::fingerprint_directory;
    use rusqlite::Connection;
    use serde_json::json;
    use std::ffi::OsStr;
    use std::fs;
    use tempfile::TempDir;

    fn pre_0025_catalog() -> MigrationCatalog {
        let latest = default_migration_catalog().expect("default migration catalog");
        MigrationCatalog::new(
            latest
                .target_versions()
                .iter()
                .filter(|version| **version < "0025")
                .map(|version| latest.migration(version).expect("migration exists").clone())
                .collect(),
        )
        .expect("build pre-0025 catalog")
    }

    /// Verifies `xtask reconcile-migrations` cleans exact legacy Effect-managed skill directories
    /// before applying migration `0025`, while preserving user-modified directories.
    #[test]
    fn reconcile_migrations_cleans_legacy_managed_skills_before_dropping_ledger() {
        let temp = TempDir::new().expect("temp dir");
        let data_dir = temp.path().join("data");
        let database_path = data_dir.join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&workspace).expect("create workspace");

        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&DatabaseLocation::path(&database_path), &pre_0025_catalog())
            .expect("bootstrap pre-0025 db");
        drop(pool);

        let clean_dir = workspace.join(".claude/skills/clean-skill");
        fs::create_dir_all(&clean_dir).expect("create clean skill dir");
        fs::write(clean_dir.join("SKILL.md"), "---\nname: clean-skill\n---\n")
            .expect("write clean SKILL.md");
        let clean_fp = fingerprint_directory(&clean_dir, &[OsStr::new(".ora-managed.json")])
            .expect("fingerprint clean dir")
            .to_string();
        fs::write(
            clean_dir.join(".ora-managed.json"),
            r#"{"schema_version":1,"resource":"res-1","managed_identity":"mi-clean"}"#,
        )
        .expect("write clean marker");

        let drifted_dir = workspace.join(".claude/skills/drifted-skill");
        fs::create_dir_all(&drifted_dir).expect("create drifted skill dir");
        fs::write(
            drifted_dir.join("SKILL.md"),
            "---\nname: drifted-skill\n---\n",
        )
        .expect("write drifted SKILL.md");
        let drifted_fp = fingerprint_directory(&drifted_dir, &[OsStr::new(".ora-managed.json")])
            .expect("fingerprint drifted dir")
            .to_string();
        fs::write(
            drifted_dir.join(".ora-managed.json"),
            r#"{"schema_version":1,"resource":"res-1","managed_identity":"mi-drift"}"#,
        )
        .expect("write drifted marker");
        fs::write(drifted_dir.join("local-edit.txt"), "user change").expect("mutate drifted dir");

        let connection = Connection::open(&database_path).expect("open sqlite");
        let locator_json = json!({ "path": workspace }).to_string();
        let descriptor_json = json!({
            "kind": "filesystem_directory_v1",
            "descriptor": {
                "workspace_root": workspace,
                "relative_path": ".claude/skills",
            }
        })
        .to_string();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");
        connection
            .execute(
                "INSERT INTO projects (id, name, created_at, updated_at, is_deleted) VALUES ('p1', 'P', 1, 1, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO workspace_locations (id, location_kind, locator_version, locator_json, created_at, updated_at)
                 VALUES ('l1', 'local_filesystem', 1, ?1, 1, 1)",
                [&locator_json],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
                 VALUES ('w1', 'p1', 'main', 'l1', 'active', 1, 1, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO effect_scopes (id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at)
                 VALUES ('scope-1', 'workspace', 'w1', 'active', 1, 1, 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO effect_sources (id, effect_kind, source_kind, namespace, identifier, lifecycle, publication_state, created_at, updated_at)
                 VALUES ('src-1', 'ora/skill', 'local', 'local', 'clean-skill', 'active', 'unpublished', 1, 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO effect_revisions (id, source_id, revision_key, definition_kind, definition_version, definition_json, digest, availability, created_at, updated_at)
                 VALUES ('rev-1', 'src-1', 'v1', 'ora/skill.v1', 1, '{}', 'sha256:rev1', 'available', 1, 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO effect_desired_effects (id, scope_id, revision_id, parameters_kind, parameters_version, parameters_json, selector_version, selector_json, created_at, updated_at)
                 VALUES ('desired-1', 'scope-1', 'rev-1', 'ora/skill', 1, '{}', 1, '{}', 1, 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO effect_resources (
                    id, scope_id, resource_key, adapter_id, descriptor_version, descriptor_json,
                    materialization_format, lifecycle, claim_fence, created_at, updated_at
                 ) VALUES (
                    'res-1', 'scope-1', 'filesystem-directory:.claude/skills', 'ora/filesystem-directory', 1, ?1,
                    'ora/skill-directory.v1', 'active', 0, 1, 1
                 )",
                [&descriptor_json],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO effect_managed_items (
                    id, scope_id, resource_id, desired_effect_id, applied_revision_id,
                    native_identity, fingerprint, applied_generation, created_at, updated_at
                 ) VALUES
                    ('mi-clean', 'scope-1', 'res-1', 'desired-1', 'rev-1', 'clean-skill', ?1, 1, 1, 1),
                    ('mi-drift', 'scope-1', 'res-1', 'desired-1', 'rev-1', 'drifted-skill', ?2, 1, 1, 1)",
                rusqlite::params![clean_fp, drifted_fp],
            )
            .unwrap();
        drop(connection);

        run_reconcile_migrations(&data_dir).expect("reconcile dev database");

        let backup_path = ora_db::pre_0025_backup_path(&database_path);
        assert!(
            backup_path.is_file(),
            "pre-0025 backup must be created before 0025 runs"
        );
        assert!(
            !clean_dir.exists(),
            "exact managed skill directory must be deleted before 0025 drops the ledger"
        );
        assert!(
            drifted_dir.exists(),
            "drifted skill directory must be preserved"
        );
        let verify = Connection::open(&database_path).expect("reopen sqlite");
        let effect_tables: i64 = verify
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'effect\\_%' ESCAPE '\\'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(effect_tables, 0);
    }
}

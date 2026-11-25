use super::{backup_database_before_0025, cleanup_legacy_managed_skills, pre_0025_backup_path};
use crate::{DatabaseBootstrapper, DatabaseLocation, MigrationCatalog, default_migration_catalog};
use ora_logging::with_trace_logging;
use ora_utils::directory::fingerprint_directory;
use pretty_assertions::assert_eq;
use rusqlite::Connection;
use serde_json::json;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
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

fn setup_pre_0025_database(database_path: &Path, workspace_root: &Path) {
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(&DatabaseLocation::path(database_path), &pre_0025_catalog())
        .expect("bootstrap pre-0025 database");
    drop(pool);

    let connection = Connection::open(database_path).expect("open pre-0025 sqlite");
    let locator_json = json!({ "path": workspace_root }).to_string();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    connection
        .execute(
            "INSERT INTO projects (id, name, created_at, updated_at, is_deleted) VALUES ('proj-1', 'Project 1', 1, 1, 0)",
            [],
        )
        .expect("insert project");
    connection
        .execute(
            "INSERT INTO workspace_locations (id, location_kind, locator_version, locator_json, created_at, updated_at)
             VALUES ('loc-1', 'local_filesystem', 1, ?1, 1, 1)",
            [&locator_json],
        )
        .expect("insert workspace location");
    connection
        .execute(
            "INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
             VALUES ('ws-1', 'proj-1', 'main', 'loc-1', 'active', 1, 1, 0)",
            [],
        )
        .expect("insert workspace");
    connection
        .execute(
            "INSERT INTO effect_scopes (id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at)
             VALUES ('scope-1', 'workspace', 'ws-1', 'active', 1, 1, 1)",
            [],
        )
        .expect("insert effect scope");
    connection
        .execute(
            "INSERT INTO effect_sources (id, effect_kind, source_kind, namespace, identifier, lifecycle, publication_state, created_at, updated_at)
             VALUES ('src-1', 'ora/skill', 'local', 'local', 'review', 'active', 'unpublished', 1, 1)",
            [],
        )
        .expect("insert effect source");
    connection
        .execute(
            "INSERT INTO effect_revisions (id, source_id, revision_key, definition_kind, definition_version, definition_json, digest, availability, created_at, updated_at)
             VALUES ('rev-1', 'src-1', 'v1', 'ora/skill.v1', 1, '{}', 'sha256:rev1', 'available', 1, 1)",
            [],
        )
        .expect("insert effect revision");
    connection
        .execute(
            "INSERT INTO effect_desired_effects (id, scope_id, revision_id, parameters_kind, parameters_version, parameters_json, selector_version, selector_json, created_at, updated_at)
             VALUES ('desired-1', 'scope-1', 'rev-1', 'ora/skill', 1, '{}', 1, '{}', 1, 1)",
            [],
        )
        .expect("insert desired effect");
}

fn insert_managed_skill_row(
    database_path: &Path,
    resource_id: &str,
    managed_identity: &str,
    workspace_root: &Path,
    relative_path: &str,
    native_identity: &str,
    fingerprint: &str,
) {
    let connection = Connection::open(database_path).expect("open sqlite");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    let descriptor_json = json!({
        "kind": "filesystem_directory_v1",
        "descriptor": {
            "workspace_root": workspace_root,
            "relative_path": relative_path,
        }
    })
    .to_string();
    let resource_key = format!("filesystem-directory:{relative_path}");
    connection
        .execute(
            "INSERT OR IGNORE INTO effect_resources (
                id, scope_id, resource_key, adapter_id, descriptor_version, descriptor_json,
                materialization_format, lifecycle, claim_fence, created_at, updated_at
             ) VALUES (
                ?1, 'scope-1', ?2, 'ora/filesystem-directory', 1, ?3,
                'ora/skill-directory.v1', 'active', 0, 1, 1
             )",
            rusqlite::params![resource_id, resource_key, descriptor_json],
        )
        .expect("insert effect resource");
    connection
        .execute(
            "INSERT INTO effect_managed_items (
                id, scope_id, resource_id, desired_effect_id, applied_revision_id,
                native_identity, fingerprint, applied_generation, created_at, updated_at
             ) VALUES (?1, 'scope-1', ?2, 'desired-1', 'rev-1', ?3, ?4, 1, 1, 1)",
            rusqlite::params![managed_identity, resource_id, native_identity, fingerprint],
        )
        .expect("insert effect managed item");
}

fn write_managed_skill_dir(
    workspace_root: &Path,
    relative_path: &str,
    native_identity: &str,
    resource_id: &str,
    managed_identity: &str,
) -> (std::path::PathBuf, String) {
    let dir = workspace_root.join(relative_path).join(native_identity);
    fs::create_dir_all(&dir).expect("create skill directory");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {native_identity}\ndescription: Managed skill\n---\n"),
    )
    .expect("write SKILL.md");
    fs::write(dir.join("notes.txt"), "helper").expect("write helper");
    let fingerprint = fingerprint_directory(&dir, &[OsStr::new(".ora-managed.json")])
        .expect("fingerprint directory")
        .to_string();
    fs::write(
        dir.join(".ora-managed.json"),
        json!({
            "schema_version": 1,
            "resource": resource_id,
            "managed_identity": managed_identity,
        })
        .to_string(),
    )
    .expect("write .ora-managed.json");
    (dir, fingerprint)
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/legacy-effect-cleanup.md`:
/// exact managed skill directories are removed before migration `0025` while parent folders,
/// sibling user directories, and `.ora/artifacts` remain intact, and post-`0025` runs are no-ops.
#[test]
fn deletes_exact_managed_skills_and_preserves_siblings_parent_and_artifacts() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        setup_pre_0025_database(&database_path, &workspace);

        let (managed_dir, fingerprint) =
            write_managed_skill_dir(&workspace, ".claude/skills", "review", "res-1", "mi-1");
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-1",
            &workspace,
            ".claude/skills",
            "review",
            &fingerprint,
        );

        let user_skill = workspace.join(".claude/skills/custom-user-skill");
        fs::create_dir_all(&user_skill).expect("create user skill");
        fs::write(
            user_skill.join("SKILL.md"),
            "---\nname: custom\ndescription: user\n---\n",
        )
        .expect("write user SKILL.md");

        let artifacts_file = workspace.join(".ora/artifacts/output.txt");
        fs::create_dir_all(artifacts_file.parent().expect("artifacts dir"))
            .expect("create .ora/artifacts");
        fs::write(&artifacts_file, "keep me").expect("write artifact");

        cleanup_legacy_managed_skills(&database_path);

        assert!(
            !managed_dir.exists(),
            "unedited managed directory must be removed"
        );
        assert!(
            workspace.join(".claude/skills").is_dir(),
            "parent .claude/skills directory must stay intact"
        );
        assert!(
            user_skill.join("SKILL.md").is_file(),
            "sibling user skill directory must stay intact"
        );
        assert_eq!(fs::read_to_string(&artifacts_file).unwrap(), "keep me");

        // Idempotent re-run before 0025 leaves everything intact without errors.
        cleanup_legacy_managed_skills(&database_path);

        // Migrate to latest (runs 0025, dropping effect_* tables) and verify no-op afterwards.
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("latest catalog"),
            )
            .expect("migrate to latest");
        drop(pool);
        cleanup_legacy_managed_skills(&database_path);
        assert!(user_skill.join("SKILL.md").is_file());
    });
}

/// Covers `specs/test-cases/desktop/core/agent/session/skill/legacy-effect-cleanup.md`:
/// user-modified contents (drift) and missing/invalid/mismatched `.ora-managed.json` markers keep
/// directories intact, while errors on earlier items never block cleanup of subsequent valid items.
#[test]
fn preserves_drifted_or_unverified_directories_and_continues_on_errors() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        setup_pre_0025_database(&database_path, &workspace);

        let (drifted_dir, drifted_fp) =
            write_managed_skill_dir(&workspace, ".claude/skills", "drifted", "res-1", "mi-drift");
        fs::write(drifted_dir.join("extra.txt"), "user edit").expect("mutate managed dir");
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-drift",
            &workspace,
            ".claude/skills",
            "drifted",
            &drifted_fp,
        );

        let (missing_marker_dir, missing_marker_fp) = write_managed_skill_dir(
            &workspace,
            ".claude/skills",
            "no-marker",
            "res-1",
            "mi-nomarker",
        );
        fs::remove_file(missing_marker_dir.join(".ora-managed.json")).expect("remove marker");
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-nomarker",
            &workspace,
            ".claude/skills",
            "no-marker",
            &missing_marker_fp,
        );

        let (wrong_marker_dir, wrong_marker_fp) = write_managed_skill_dir(
            &workspace,
            ".claude/skills",
            "wrong-marker",
            "res-1",
            "mi-wrong",
        );
        fs::write(
            wrong_marker_dir.join(".ora-managed.json"),
            r#"{"schema_version":1,"resource":"res-1","managed_identity":"other-id"}"#,
        )
        .expect("overwrite marker with mismatched identity");
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-wrong",
            &workspace,
            ".claude/skills",
            "wrong-marker",
            &wrong_marker_fp,
        );

        let (malformed_marker_dir, malformed_marker_fp) = write_managed_skill_dir(
            &workspace,
            ".claude/skills",
            "bad-json",
            "res-1",
            "mi-badjson",
        );
        fs::write(malformed_marker_dir.join(".ora-managed.json"), "{not-json")
            .expect("corrupt marker");
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-badjson",
            &workspace,
            ".claude/skills",
            "bad-json",
            &malformed_marker_fp,
        );

        // Lexicographically ordered after the failing items so we prove cleanup continues past them.
        let (clean_dir, clean_fp) =
            write_managed_skill_dir(&workspace, ".claude/skills", "z-clean", "res-1", "mi-clean");
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-clean",
            &workspace,
            ".claude/skills",
            "z-clean",
            &clean_fp,
        );

        cleanup_legacy_managed_skills(&database_path);

        assert!(drifted_dir.exists(), "drifted skill dir must be preserved");
        assert!(
            missing_marker_dir.exists(),
            "markerless dir must be preserved"
        );
        assert!(
            wrong_marker_dir.exists(),
            "mismatched marker dir must be preserved"
        );
        assert!(
            malformed_marker_dir.exists(),
            "malformed marker dir must be preserved"
        );
        assert!(
            !clean_dir.exists(),
            "subsequent valid managed dir must still be cleaned after earlier skips"
        );
    });
}

/// Verifies missing database files are skipped without creating empty sqlite files on disk.
#[test]
fn skips_missing_database_without_creating_file() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let missing = temp.path().join("does-not-exist.sqlite3");
        cleanup_legacy_managed_skills(&missing);
        assert!(!missing.exists());
    });
}

/// Covers symlink/junction refusal so cleanup never deletes or traverses a linked skill directory
/// or linked resource ancestor.
#[cfg(windows)]
#[test]
fn skips_windows_junction_targets_without_deleting_link_or_target() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside-target");
        fs::create_dir_all(workspace.join(".claude").join("skills")).expect("create skills root");
        fs::create_dir_all(&outside).expect("create outside target");
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: linked\ndescription: target\n---\n",
        )
        .expect("write outside SKILL.md");
        let outside_fp = fingerprint_directory(&outside, &[OsStr::new(".ora-managed.json")])
            .expect("fingerprint outside")
            .to_string();
        fs::write(
            outside.join(".ora-managed.json"),
            r#"{"schema_version":1,"resource":"res-1","managed_identity":"mi-link"}"#,
        )
        .expect("write marker inside outside target");

        let junction = workspace.join(".claude").join("skills").join("linked");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .expect("run mklink /J");
        assert!(
            output.status.success(),
            "mklink /J failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        setup_pre_0025_database(&database_path, &workspace);
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-link",
            &workspace,
            ".claude/skills",
            "linked",
            &outside_fp,
        );

        cleanup_legacy_managed_skills(&database_path);

        assert!(junction.exists(), "junction link must be left intact");
        assert!(
            outside.join("SKILL.md").is_file(),
            "junction target contents must not be deleted"
        );
    });
}

#[cfg(unix)]
#[test]
fn skips_unix_symlink_targets_without_deleting_link_or_target() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside-target");
        fs::create_dir_all(workspace.join(".claude/skills")).expect("create skills root");
        fs::create_dir_all(&outside).expect("create outside target");
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: linked\ndescription: target\n---\n",
        )
        .expect("write outside SKILL.md");
        let outside_fp = fingerprint_directory(&outside, &[OsStr::new(".ora-managed.json")])
            .expect("fingerprint outside")
            .to_string();
        fs::write(
            outside.join(".ora-managed.json"),
            r#"{"schema_version":1,"resource":"res-1","managed_identity":"mi-link"}"#,
        )
        .expect("write marker inside outside target");

        let symlink = workspace.join(".claude/skills/linked");
        std::os::unix::fs::symlink(&outside, &symlink).expect("create symlink");

        setup_pre_0025_database(&database_path, &workspace);
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-link",
            &workspace,
            ".claude/skills",
            "linked",
            &outside_fp,
        );

        cleanup_legacy_managed_skills(&database_path);

        assert!(fs::symlink_metadata(&symlink).is_ok());
        assert!(outside.join("SKILL.md").is_file());
    });
}

/// Verifies marker `resource` validation independently of `managed_identity` and fingerprint:
/// a directory whose `.ora-managed.json` has the matching `managed_identity` and valid content
/// fingerprint, but a different `resource` ID, must be preserved.
#[test]
fn preserves_managed_skill_when_marker_resource_differs() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        setup_pre_0025_database(&database_path, &workspace);

        let (dir, fingerprint) = write_managed_skill_dir(
            &workspace,
            ".claude/skills",
            "wrong-resource",
            "res-other",
            "mi-same-identity",
        );
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-same-identity",
            &workspace,
            ".claude/skills",
            "wrong-resource",
            &fingerprint,
        );

        cleanup_legacy_managed_skills(&database_path);

        assert!(
            dir.exists(),
            "directory with mismatched marker resource ID must be preserved"
        );
        assert!(dir.join("SKILL.md").is_file());
    });
}

/// Verifies ancestor-link rejection independently of workspace containment: when a resource
/// ancestor (`.claude/skills`) is a junction/symlink pointing to another directory *inside* the
/// same workspace, canonical containment still holds so ONLY the explicit ancestor-link check
/// preserves the linked target directory.
#[test]
fn preserves_managed_skill_when_ancestor_directory_is_a_link() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        let real_skills_parent = workspace.join("real-skills");
        fs::create_dir_all(workspace.join(".claude")).expect("create .claude");
        fs::create_dir_all(&real_skills_parent).expect("create in-workspace target");

        let (target_skill_dir, fingerprint) = write_managed_skill_dir(
            &workspace,
            "real-skills",
            "ancestor-linked",
            "res-1",
            "mi-ancestor",
        );

        let linked_ancestor = workspace.join(".claude").join("skills");
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&linked_ancestor)
                .arg(&real_skills_parent)
                .output()
                .expect("run mklink /J for ancestor");
            assert!(
                output.status.success(),
                "mklink /J failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_skills_parent, &linked_ancestor)
                .expect("create ancestor symlink");
        }

        setup_pre_0025_database(&database_path, &workspace);
        insert_managed_skill_row(
            &database_path,
            "res-1",
            "mi-ancestor",
            &workspace,
            ".claude/skills",
            "ancestor-linked",
            &fingerprint,
        );

        cleanup_legacy_managed_skills(&database_path);

        assert!(
            fs::symlink_metadata(&linked_ancestor).is_ok(),
            "ancestor link must be preserved"
        );
        assert!(
            target_skill_dir.join("SKILL.md").is_file(),
            "skill directory reached through an ancestor link must be preserved"
        );
    });
}

/// Verifies Owner Decision B: an existing pre-`0025` database (including WAL-mode writes) is
/// backed up to `ora.sqlite3.pre-0025.bak` before `0025` runs, the backup opens cleanly under a
/// pre-`0025` catalog with all data intact, retries preserve an existing backup untouched, and
/// missing, empty, or post-`0025` databases create no backup.
#[test]
fn creates_self_contained_pre_0025_backup_once_and_skips_fresh_or_post_0025_databases() {
    with_trace_logging(|| {
        let temp = TempDir::new().expect("temp dir");
        let missing = temp.path().join("missing.sqlite3");
        assert_eq!(backup_database_before_0025(&missing).unwrap(), None);
        assert!(!missing.exists());
        assert!(!pre_0025_backup_path(&missing).exists());

        let empty = temp.path().join("empty.sqlite3");
        fs::write(&empty, b"").expect("create empty file");
        assert_eq!(backup_database_before_0025(&empty).unwrap(), None);
        assert!(!pre_0025_backup_path(&empty).exists());

        let database_path = temp.path().join("ora.sqlite3");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        setup_pre_0025_database(&database_path, &workspace);

        // Write additional data in WAL mode without truncating the WAL to verify `VACUUM INTO`
        // produces a self-contained single-file backup.
        let writer = Connection::open(&database_path).expect("open pre-0025 db");
        writer
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 INSERT INTO projects (id, name, created_at, updated_at, is_deleted)
                 VALUES ('proj-wal', 'WAL Project', 2, 2, 0);",
            )
            .expect("write WAL row");
        drop(writer);

        let expected_backup = pre_0025_backup_path(&database_path);
        assert_eq!(
            expected_backup.file_name().and_then(OsStr::to_str),
            Some("ora.sqlite3.pre-0025.bak")
        );
        let created = backup_database_before_0025(&database_path)
            .expect("create pre-0025 backup")
            .expect("pre-0025 database must produce a backup");
        assert_eq!(created, expected_backup);
        assert!(expected_backup.is_file());
        assert!(
            !temp.path().join("ora.sqlite3.pre-0025.bak.tmp").exists(),
            "temporary backup file must be renamed away"
        );

        // Verify the standalone backup opens cleanly on a pre-0025 catalog and retains WAL data.
        let restored_copy = temp.path().join("restored-old-build.sqlite3");
        fs::copy(&expected_backup, &restored_copy).expect("copy backup for recovery test");
        let restored_pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&DatabaseLocation::path(&restored_copy), &pre_0025_catalog())
            .expect("older build must open restored pre-0025 backup cleanly");
        drop(restored_pool);
        let verify_backup = Connection::open(&restored_copy).expect("open restored backup");
        let wal_project_count: i64 = verify_backup
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id = 'proj-wal'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wal_project_count, 1);
        let effect_scope_count: i64 = verify_backup
            .query_row("SELECT COUNT(*) FROM effect_scopes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(effect_scope_count, 1);
        drop(verify_backup);

        // Calling backup again while still pre-0025 preserves the existing backup untouched.
        let original_bytes = fs::read(&expected_backup).expect("read original backup");
        let mutator = Connection::open(&database_path).expect("reopen pre-0025 db");
        mutator
            .execute(
                "INSERT INTO projects (id, name, created_at, updated_at, is_deleted)
                 VALUES ('proj-after-backup', 'After Backup', 3, 3, 0)",
                [],
            )
            .unwrap();
        drop(mutator);
        assert_eq!(
            backup_database_before_0025(&database_path).unwrap(),
            Some(expected_backup.clone())
        );
        assert_eq!(
            fs::read(&expected_backup).expect("re-read original backup"),
            original_bytes
        );

        // Upgrade to latest (applies 0025), remove the backup, and verify post-0025 runs do NOT
        // create another pre-0025 backup.
        let upgraded = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("latest catalog"),
            )
            .expect("upgrade through 0025");
        drop(upgraded);
        fs::remove_file(&expected_backup).expect("remove backup after upgrade");
        assert_eq!(backup_database_before_0025(&database_path).unwrap(), None);
        assert!(!expected_backup.exists());
    });
}

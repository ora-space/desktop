use super::unfinished_fixture;
use crate::{
    DatabaseBootstrapper, DatabaseLocation, MigrationCatalog, RepositoryPool,
    SqliteEffectRepository, default_migration_catalog, test_clock::TestClock,
};
use ora_effect::{EffectRepository, LocalTimestamp, OperationProgress};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::types::Value;
use std::collections::BTreeMap;

/// Reads complete durable business evidence while excluding audit values whose rollback semantics differ.
fn business_snapshot(
    pool: &RepositoryPool,
) -> Result<BTreeMap<String, Vec<Vec<Value>>>, crate::DatabaseError> {
    pool.with_connection(|connection| {
        let tables = connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'effect_%' ORDER BY name")?
            .query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        let mut snapshot = BTreeMap::new();
        for table in tables {
            let mut statement = connection.prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))?;
            let columns = statement.column_names().iter().enumerate().filter_map(|(index, name)| (*name != "updated_at").then_some(index)).collect::<Vec<_>>();
            let rows = statement.query_map([], |row| columns.iter().map(|index| row.get::<_, Value>(*index)).collect::<Result<Vec<_>, _>>())?.collect::<Result<Vec<_>, _>>()?;
            snapshot.insert(table, rows);
        }
        Ok(snapshot)
    })
}

/// Upgrade and rollback must preserve recovery evidence even after audit-only writes changed the row.
/// specs/test-cases/desktop/core/effect/time.md#migration-round-trips-preserve-recovery-evidence
#[test]
fn migration_round_trips_preserve_recovery_evidence() -> Result<(), Box<dyn std::error::Error>> {
    with_trace_logging(|| {
        let (directory, pool, target, resource) = unfinished_fixture();
        let root = directory.path().join("package");
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("SKILL.md"), b"manifest")?;
        crate::SqliteSkillRepository::with_clock(pool.clone(), TestClock::new(20))
            .replace_plugin_skills(
                &ora_domain::PluginId::new("official", "review")?,
                "1",
                &[crate::PluginSkillProjection {
                    name: "review".to_string(),
                    description: "Reviews changes".to_string(),
                    package_fingerprint: super::package_fingerprint(&root),
                    package_root: root,
                    skill_md_digest: ora_effect::Digest::sha256(b"manifest"),
                }],
                /*updated_at*/ 10,
            )?;
        pool.with_connection(|connection| {
            connection.execute("INSERT INTO effect_managed_items
                (id, scope_id, resource_id, desired_effect_id, applied_revision_id, native_identity, fingerprint, applied_generation, created_at, updated_at)
                SELECT 'existing-owner', resource.scope_id, resource.id, desired.id, desired.revision_id, 'existing-review', ?2, 0, 10, 10
                FROM effect_resources resource JOIN effect_desired_effects desired ON desired.scope_id = resource.scope_id WHERE resource.id = ?1",
                rusqlite::params![resource, ora_effect::Fingerprint::sha256(b"owned").as_str()])?;
            connection.execute("INSERT INTO effect_operation_artifacts
                (id, operation_id, role, locator_version, locator_json, expected_fingerprint, state, created_at, updated_at)
                VALUES ('artifact', 'operation-1', 'staging', 1, ?1, ?2, 'reserved', 11, 11)",
                rusqlite::params![serde_json::to_string(&ora_effect::VersionedResourceLocator::FilesystemPathV1(directory.path().join("staging")))?, ora_effect::Fingerprint::sha256(b"artifact").as_str()])?;
            Ok(())
        })?;
        let repository = SqliteEffectRepository::with_clock(pool.clone(), TestClock::new(200));
        assert_eq!(
            repository.quarantine_unfinished_operations(LocalTimestamp::from_millis(101))?,
            1
        );
        let operations = repository.load_unfinished_operations()?;
        assert_eq!(
            operations[0].progress(),
            &OperationProgress::RecoveryRequired {
                prepared_at: LocalTimestamp::from_millis(11),
                applied_at: None,
                detected_at: LocalTimestamp::from_millis(101),
            }
        );
        let workspace = crate::SqliteWorkspaceRepository::new(pool.clone())
            .find_workspace(&ora_domain::WorkspaceId::new("workspace-1"))?
            .expect("fixture Workspace");
        repository.declare_consumer(&super::declaration("official/second"), &[workspace])?;
        assert_eq!(
            repository
                .claim_due_targets(
                    &ora_effect::WorkerIdentity::parse("second-worker")?,
                    LocalTimestamp::from_millis(201),
                    LocalTimestamp::from_millis(1000),
                    /*limit*/ 1
                )?
                .len(),
            1
        );
        let before = business_snapshot(&pool)?;
        let latest = default_migration_catalog()?;
        let previous = MigrationCatalog::with_target_versions(
            latest
                .target_versions()
                .iter()
                .map(|version| {
                    latest
                        .migration(version)
                        .expect("registered migration")
                        .clone()
                })
                .collect(),
            latest
                .target_versions()
                .iter()
                .copied()
                .filter(|version| *version != "0010")
                .collect(),
        )?;
        let location = DatabaseLocation::path(directory.path().join("ora.sqlite"));
        let bootstrap = DatabaseBootstrapper::new(TestClock::new(300));
        for _ in 0..2 {
            let old_pool = bootstrap.bootstrap_repository_pool(&location, &previous)?;
            let detection = old_pool.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT updated_at FROM effect_operations WHERE id = 'operation-1'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(Into::into)
            })?;
            assert_eq!(detection, 101);
            let upgraded = bootstrap.bootstrap_repository_pool(&location, &latest)?;
            assert_eq!(business_snapshot(&upgraded)?, before);
            let reopened =
                SqliteEffectRepository::with_clock(upgraded.clone(), TestClock::new(400));
            assert_eq!(reopened.load_unfinished_operations()?, operations);
            assert_eq!(
                reopened
                    .load_target_status(&target)?
                    .expect("recovered Target")
                    .conditions
                    .len(),
                1
            );
            upgraded.with_connection(|connection| {
                assert!(connection.execute("UPDATE effect_operations SET detected_at = NULL WHERE id = 'operation-1'", []).is_err());
                assert!(connection.execute("UPDATE effect_operations SET detected_at = 10 WHERE id = 'operation-1'", []).is_err());
                assert!(connection.execute("UPDATE effect_operations SET phase = 'prepared' WHERE id = 'operation-1'", []).is_err());
                assert_eq!(connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| row.get::<_, i64>(0))?, 0);
                Ok(())
            })?;
        }
        Ok(())
    })
}

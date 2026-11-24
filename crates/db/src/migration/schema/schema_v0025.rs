use super::Migration;

// Schema v0025 backfills plugin skill `package_root` onto `skills` before clearing circular FKs
// (`effect_sources.published_revision_id`, `effect_consumers.current_revision_id`) and dropping
// every Effect trigger and table in child-before-parent order.
const UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE skills ADD COLUMN package_root TEXT;",
    "UPDATE skills
     SET package_root = (
         SELECT json_extract(r.definition_json, '$.definition.package_root')
         FROM effect_sources e
         JOIN effect_revisions r ON r.id = e.published_revision_id
         WHERE e.effect_kind = 'ora/skill'
           AND e.source_kind = 'plugin'
           AND e.namespace = skills.namespace COLLATE NOCASE
           AND e.identifier = skills.name COLLATE NOCASE
     )
     WHERE package_root IS NULL;",
    "PRAGMA defer_foreign_keys = ON;",
    "UPDATE effect_sources SET publication_state = 'unpublished', published_revision_id = NULL;",
    "DROP TRIGGER IF EXISTS effect_scopes_after_workspace_insert;",
    "DROP TRIGGER IF EXISTS effect_audit_events_no_update;",
    "DROP TRIGGER IF EXISTS effect_readiness_receipts_no_update;",
    "DROP TRIGGER IF EXISTS effect_coordination_receipts_no_update;",
    "DROP TRIGGER IF EXISTS effect_operation_artifacts_authority_immutable;",
    "DROP TRIGGER IF EXISTS effect_operations_recovery_time_insert;",
    "DROP TRIGGER IF EXISTS effect_operations_recovery_time_update;",
    "DROP TRIGGER IF EXISTS effect_operations_intent_immutable;",
    "DROP TRIGGER IF EXISTS effect_attempt_resource_projections_no_update;",
    "DROP TRIGGER IF EXISTS effect_reconcile_attempts_inputs_immutable;",
    "DROP TRIGGER IF EXISTS effect_resolved_materializations_no_update;",
    "DROP TRIGGER IF EXISTS effect_resource_projection_contributors_no_update;",
    "DROP TRIGGER IF EXISTS effect_resource_projections_no_update;",
    "DROP TRIGGER IF EXISTS effect_resource_requirement_effects_no_update;",
    "DROP TRIGGER IF EXISTS effect_resource_requirements_no_update;",
    "DROP TRIGGER IF EXISTS effect_target_projection_effects_no_update;",
    "DROP TRIGGER IF EXISTS effect_target_projections_no_update;",
    "DROP TRIGGER IF EXISTS effect_consumer_revisions_no_update;",
    "DROP TRIGGER IF EXISTS effect_revisions_immutable;",
    "DROP TABLE IF EXISTS effect_audit_events;",
    "DROP TABLE IF EXISTS effect_readiness_receipts;",
    "DROP TABLE IF EXISTS effect_operation_artifacts;",
    "DROP TABLE IF EXISTS effect_resource_status;",
    "DROP TABLE IF EXISTS effect_target_status;",
    "DROP TABLE IF EXISTS effect_operations;",
    "DROP TABLE IF EXISTS effect_coordination_receipts;",
    "DROP TABLE IF EXISTS effect_attempt_resource_projections;",
    "DROP TABLE IF EXISTS effect_reconcile_attempts;",
    "DROP TABLE IF EXISTS effect_resource_claims;",
    "DROP TABLE IF EXISTS effect_reconcile_requests;",
    "DROP TABLE IF EXISTS effect_conditions;",
    "DROP TABLE IF EXISTS effect_managed_items;",
    "DROP TABLE IF EXISTS effect_resolved_materializations;",
    "DROP TABLE IF EXISTS effect_resource_projection_contributors;",
    "DROP TABLE IF EXISTS effect_resource_projections;",
    "DROP TABLE IF EXISTS effect_resource_requirement_effects;",
    "DROP TABLE IF EXISTS effect_resource_requirements;",
    "DROP TABLE IF EXISTS effect_target_projection_effects;",
    "DROP TABLE IF EXISTS effect_target_projections;",
    "DROP TABLE IF EXISTS effect_target_resource_bindings;",
    "DROP TABLE IF EXISTS effect_resources;",
    "DROP TABLE IF EXISTS effect_target_declarations;",
    "DROP TABLE IF EXISTS effect_targets;",
    "DELETE FROM effect_consumers;",
    "DELETE FROM effect_consumer_revisions;",
    "DROP TABLE IF EXISTS effect_consumer_revisions;",
    "DROP TABLE IF EXISTS effect_consumers;",
    "DROP TABLE IF EXISTS effect_desired_effects;",
    "DROP TABLE IF EXISTS effect_revisions;",
    "DROP TABLE IF EXISTS effect_sources;",
    "DROP TABLE IF EXISTS effect_scopes;",
];

/// Intentionally empty: the Effect ledger and pre-migration filesystem ownership cannot be
/// meaningfully restored once D9 cleanup and this drop have executed.
const DOWN_STATEMENTS: &[&str] = &[];

/// Retires all Effect ledger tables/triggers while preserving plugin skill `package_root`s.
pub fn migration() -> Migration {
    Migration::new("0025", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use crate::migration::reconcile_database_versions;
    use crate::{MigrationCatalog, default_migration_catalog, test_clock::TestClock};
    use ora_logging::with_trace_logging;
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;

    fn pre_0025_catalog() -> MigrationCatalog {
        let latest = default_migration_catalog().unwrap();
        MigrationCatalog::new(
            latest
                .target_versions()
                .iter()
                .filter(|version| **version < "0025")
                .map(|version| latest.migration(version).unwrap().clone())
                .collect(),
        )
        .unwrap()
    }

    /// Backfills `skills.package_root` from legacy `effect_sources`/`effect_revisions` for plugin
    /// skills while keeping local skills `NULL`, even when circular Effect FKs are populated and
    /// `PRAGMA foreign_keys = ON`.
    ///
    /// Covers `specs/test-cases/desktop/core/agent/session/skill/legacy-effect-cleanup.md`.
    #[test]
    fn backfills_plugin_package_root_and_drops_populated_effect_ledger() {
        with_trace_logging(|| {
            let latest = default_migration_catalog().unwrap();
            let previous = pre_0025_catalog();
            let mut connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch("PRAGMA foreign_keys = ON;")
                .unwrap();
            reconcile_database_versions(&mut connection, &previous, &TestClock::new(1)).unwrap();

            connection
                .execute_batch(
                    "BEGIN;
                     INSERT INTO projects (id, name, created_at, updated_at, is_deleted)
                         VALUES ('proj-1', 'Project 1', 1, 1, 0);
                     INSERT INTO workspace_locations (id, location_kind, locator_version, locator_json, created_at, updated_at)
                         VALUES ('loc-1', 'local_filesystem', 1, '{\"path\":\"/tmp/ws-1\"}', 1, 1);
                     INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
                         VALUES ('ws-1', 'proj-1', 'main', 'loc-1', 'active', 1, 1, 0);
                     INSERT INTO skills (id, namespace, name, description, created_at, updated_at, is_deleted) VALUES
                         ('skill-local', 'local', 'alpha', 'Local skill', 1, 1, 0),
                         ('plugin:official/review:audit', 'official/review', 'audit', 'Plugin skill', 1, 1, 0);
                     INSERT INTO effect_scopes (id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at)
                         VALUES ('workspace:ws-1', 'workspace', 'ws-1', 'active', 1, 1, 1);
                     INSERT INTO effect_sources (id, effect_kind, source_kind, namespace, identifier, lifecycle, publication_state, created_at, updated_at)
                         VALUES ('src-plugin', 'ora/skill', 'plugin', 'official/review', 'audit', 'active', 'unpublished', 1, 1);
                     INSERT INTO effect_revisions (id, source_id, revision_key, definition_kind, definition_version, definition_json, digest, availability, created_at, updated_at) VALUES (
                         'rev-plugin',
                         'src-plugin',
                         '1.0.0',
                         'ora/skill.v1',
                         1,
                         '{\"definition\":{\"package_root\":\"/ora/plugins/installed/official/review/1.0.0/assets/audit\"}}',
                         'sha256:abc',
                         'available',
                         1,
                         1
                     );
                     UPDATE effect_sources
                     SET publication_state = 'published', published_revision_id = 'rev-plugin'
                     WHERE id = 'src-plugin';
                     INSERT INTO effect_consumers (id, consumer_kind, identity_key, adapter_id, current_revision_id, lifecycle, created_at, updated_at)
                         VALUES ('consumer-1', 'agent_plugin', 'official/agent', 'ora/agent-plugin', 'crev-1', 'declared', 1, 1);
                     INSERT INTO effect_consumer_revisions (id, consumer_id, capability_version, capabilities_json, declaration_digest, created_at)
                         VALUES ('crev-1', 'consumer-1', 1, '{}', 'sha256:def', 1);
                     COMMIT;",
                )
                .unwrap();

            reconcile_database_versions(&mut connection, &latest, &TestClock::new(2)).unwrap();

            let rows = connection
                .prepare("SELECT id, package_root FROM skills ORDER BY id")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                rows,
                vec![
                    (
                        "plugin:official/review:audit".to_string(),
                        Some(
                            "/ora/plugins/installed/official/review/1.0.0/assets/audit".to_string()
                        )
                    ),
                    ("skill-local".to_string(), None),
                ]
            );
        });
    }

    /// Verifies no table, index, view, or trigger named or referencing `effect_` survives 0025.
    ///
    /// Covers `specs/test-cases/desktop/core/agent/session/skill/legacy-effect-cleanup.md`.
    #[test]
    fn drops_all_effect_tables_triggers_and_indexes_from_sqlite_master() {
        with_trace_logging(|| {
            let latest = default_migration_catalog().unwrap();
            let mut connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch("PRAGMA foreign_keys = ON;")
                .unwrap();
            reconcile_database_versions(&mut connection, &latest, &TestClock::new(1)).unwrap();

            let remaining = connection
                .prepare(
                    "SELECT name FROM sqlite_master
                     WHERE name LIKE 'effect\\_%' ESCAPE '\\'
                        OR sql LIKE '%effect\\_%' ESCAPE '\\'",
                )
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                remaining.is_empty(),
                "leftover effect objects: {remaining:?}"
            );
        });
    }

    /// Verifies a fresh database migrates cleanly through 0025 and allows workspace inserts
    /// without the retired `effect_scopes_after_workspace_insert` trigger.
    ///
    /// Covers `specs/test-cases/desktop/core/agent/session/skill/legacy-effect-cleanup.md`.
    #[test]
    fn fresh_database_migrates_and_inserts_workspaces_cleanly() {
        with_trace_logging(|| {
            let latest = default_migration_catalog().unwrap();
            let mut connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch("PRAGMA foreign_keys = ON;")
                .unwrap();
            reconcile_database_versions(&mut connection, &latest, &TestClock::new(1)).unwrap();

            connection
                .execute_batch(
                    "INSERT INTO projects (id, name, created_at, updated_at, is_deleted)
                         VALUES ('p1', 'Proj', 1, 1, 0);
                     INSERT INTO workspace_locations (id, location_kind, locator_version, locator_json, created_at, updated_at)
                         VALUES ('l1', 'local_filesystem', 1, '{\"path\":\"/tmp/p1\"}', 1, 1);
                     INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
                         VALUES ('w1', 'p1', 'main', 'l1', 'active', 1, 1, 0);",
                )
                .unwrap();
        });
    }
}

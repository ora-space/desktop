use crate::migration::reconcile_database_versions;
use crate::{MigrationCatalog, default_migration_catalog, test_clock::TestClock};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::{Connection, params};

/// Adopting the fork's colliding version retains active scopes and installs upstream columns.
#[test]
fn adopts_legacy_loop_version_without_rolling_back_history() {
    with_trace_logging(|| {
        let (mut connection, latest, _) = fixture();
        let before_loop = MigrationCatalog::new(
            latest
                .target_versions()
                .iter()
                .filter(|version| **version < "0011")
                .map(|version| latest.migration(version).unwrap().clone())
                .collect(),
        )
        .unwrap();
        reconcile_database_versions(&mut connection, &before_loop, &TestClock::new(2)).unwrap();
        let scopes = latest.migration("0013").unwrap();
        let old_sql = scopes.up_sql().replace(
            "ON workflow_node_runs(scope_id, node_id, COALESCE(iteration, -1))",
            "ON workflow_node_runs(scope_id, node_id)",
        );
        connection.execute_batch(&old_sql).unwrap();
        connection
            .execute(
                "INSERT INTO migrations VALUES ('0011', ?1, ?2, 3)",
                params![old_sql, scopes.down_sql()],
            )
            .unwrap();
        connection.execute_batch(r#"
            INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at)
            VALUES ('loop', 'run', 'loop', 'loop', 1, 3, 3);
            INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at)
            VALUES ('round', 'run', 'loop', 1, 1, '{"draft":"keep"}', 3, 3);
            INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, output, created_at, updated_at)
            VALUES ('writer', 'run', 'round', 'writer', 'agent', 1, 'partial', 3, 3);
        "#).unwrap();
        for _ in 0..2 {
            reconcile_database_versions(&mut connection, &latest, &TestClock::new(4)).unwrap();
            crate::migration::reconcile_database(&mut connection, &latest, &TestClock::new(5))
                .unwrap();
        }
        let retained: (String, i64, String, i64, Option<u32>) = connection.query_row(
            "SELECT scope.state, scope.status, node.output, node.status, node.iteration
             FROM workflow_execution_scopes scope JOIN workflow_node_runs node ON node.scope_id = scope.id
             WHERE node.id = 'writer'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).unwrap();
        assert_eq!(
            retained,
            (r#"{"draft":"keep"}"#.into(), 1, "partial".into(), 1, None)
        );
        let versions = connection
            .prepare("SELECT version FROM migrations ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(versions, latest.target_versions());
        connection
            .prepare("SELECT mcp_selection FROM sessions")
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM workflow_scope_downgrade_archive",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    });
}

/// Uses the production catalog and startup reconciler, including persisted downgrade statements.
fn fixture() -> (Connection, MigrationCatalog, MigrationCatalog) {
    let latest = default_migration_catalog().unwrap();
    let versions: Vec<_> = latest
        .target_versions()
        .iter()
        .copied()
        .filter(|version| *version < "0013")
        .collect();
    let previous = MigrationCatalog::new(
        versions
            .iter()
            .map(|version| latest.migration(version).unwrap().clone())
            .collect(),
    )
    .unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
    reconcile_database_versions(&mut connection, &previous, &TestClock::new(1)).unwrap();
    connection.execute_batch(r#"
        INSERT INTO projects (id, name, created_at, updated_at) VALUES ('project', 'Project', 1, 1);
        INSERT INTO workspace_locations (id, location_kind, locator_json, created_at, updated_at)
        VALUES ('location', 'local_filesystem', '{}', 1, 1);
        INSERT INTO workspaces (id, project_id, workspace_kind, location_id, created_at, updated_at)
        VALUES ('workspace', 'project', 'main', 'location', 1, 1);
        INSERT INTO workflows (id, name, created_at, updated_at) VALUES ('workflow', 'Workflow', 1, 1);
        INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at)
        VALUES ('snapshot', 'workflow', '1', '{}', 1);
        INSERT INTO workflow_runs (id, workspace_id, workflow_id, snapshot_id, name, run_status, created_at, updated_at)
        VALUES ('run', 'workspace', 'workflow', 'snapshot', 'Run', 1, 1, 2);
        INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, output, created_at, updated_at)
        VALUES ('start', 'run', 'start', 'start', 2, 'input', 1, 2);
    "#).unwrap();
    (connection, latest, previous)
}

fn pre_0025_catalog(latest: &MigrationCatalog) -> MigrationCatalog {
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

/// Flat data survives both migration directions and gains default root identity for future nodes.
#[test]
fn backfills_flat_nodes_and_preserves_old_rows() {
    with_trace_logging(|| {
        let (mut connection, latest, previous) = fixture();
        let target = pre_0025_catalog(&latest);
        reconcile_database_versions(&mut connection, &target, &TestClock::new(3)).unwrap();
        let row: (String, String, i64, String) = connection.query_row(
            "SELECT scope_id, node_id, status, output FROM workflow_node_runs WHERE id = 'start'", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(row, ("root:run".into(), "start".into(), 2, "input".into()));
        connection.execute("INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at) VALUES ('agent', 'run', 'agent', 'agent', 1, 3, 3)", []).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT scope_id FROM workflow_node_runs WHERE id = 'agent'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "root:run"
        );
        reconcile_database_versions(&mut connection, &previous, &TestClock::new(4)).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT run_status FROM workflow_runs WHERE id = 'run'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        reconcile_database_versions(&mut connection, &target, &TestClock::new(5)).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM workflow_node_runs WHERE is_deleted = 0",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    });
}

/// SQLite prevents duplicate dispatch within a round, duplicate rounds, and foreign parentage.
#[test]
fn enforces_scope_identity_and_round_uniqueness() {
    with_trace_logging(|| {
        let (mut connection, latest, _) = fixture();
        reconcile_database_versions(&mut connection, &latest, &TestClock::new(3)).unwrap();
        connection.execute("INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at) VALUES ('loop', 'run', 'loop', 'loop', 1, 3, 3)", []).unwrap();
        connection.execute("INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at) VALUES ('round', 'run', 'loop', 1, 1, '{}', 3, 3)", []).unwrap();
        for sql in [
            "INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at) VALUES ('concurrent-round', 'run', 'loop', 2, 1, '{}', 3, 3)",
            "INSERT INTO workflow_execution_scopes SELECT 'duplicate', run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at FROM workflow_execution_scopes WHERE id = 'round'",
            "INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at) VALUES ('wrong-parent', 'run', 'start', 1, 1, '{}', 3, 3)",
            "UPDATE workflow_execution_scopes SET round_index = 2 WHERE id = 'round'",
            "UPDATE workflow_run_root_scopes SET scope_id = 'round' WHERE run_id = 'run'",
            "UPDATE workflow_node_runs SET scope_id = 'round' WHERE id = 'start'",
        ] {
            assert!(connection.execute(sql, []).is_err(), "accepted {sql}");
        }
        connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, created_at, updated_at) VALUES ('writer-1', 'run', 'round', 'writer', 'agent', 1, 3, 3)", []).unwrap();
        assert!(connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, created_at, updated_at) VALUES ('writer-duplicate', 'run', 'round', 'writer', 'agent', 1, 3, 3)", []).is_err());
        connection
            .execute(
                "UPDATE workflow_execution_scopes SET status = 2 WHERE id = 'round'",
                [],
            )
            .unwrap();
        assert!(connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, created_at, updated_at) VALUES ('late-writer', 'run', 'round', 'late', 'agent', 1, 3, 3)", []).is_err());
        connection.execute("INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at) VALUES ('next-round', 'run', 'loop', 2, 1, '{}', 4, 4)", []).unwrap();
        connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, created_at, updated_at) VALUES ('writer-2', 'run', 'next-round', 'writer', 'agent', 1, 4, 4)", []).unwrap();
        connection.execute("INSERT INTO workflow_runs (id, workspace_id, workflow_id, snapshot_id, name, run_status, created_at, updated_at) VALUES ('other-run', 'workspace', 'workflow', 'snapshot', 'Other', 1, 4, 4)", []).unwrap();
        assert!(connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, created_at, updated_at) VALUES ('foreign', 'other-run', 'next-round', 'writer', 'agent', 1, 4, 4)", []).is_err());
        assert!(connection.execute("INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at) VALUES ('foreign-round', 'other-run', 'loop', 3, 1, '{}', 4, 4)", []).is_err());
        connection.execute("INSERT INTO workflow_execution_scopes (id, run_id, created_at, updated_at) VALUES ('new-root', 'run', 5, 5)", []).unwrap();
        connection
            .execute(
                "UPDATE workflow_run_root_scopes SET scope_id = 'new-root' WHERE run_id = 'run'",
                [],
            )
            .unwrap();
        assert!(connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, status, created_at, updated_at) VALUES ('stale', 'run', 'root:run', 'late', 'agent', 1, 5, 5)", []).is_err());
    });
}

/// The actual older startup path archives original evidence, settles active runs, and hides children.
#[test]
fn downgrade_archives_loop_history_and_reupgrade_is_safe() {
    with_trace_logging(|| {
        let (mut connection, latest, previous) = fixture();
        let target = pre_0025_catalog(&latest);
        reconcile_database_versions(&mut connection, &target, &TestClock::new(3)).unwrap();
        connection.execute("INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at) VALUES ('loop', 'run', 'loop', 'loop', 1, 3, 3)", []).unwrap();
        connection.execute("INSERT INTO workflow_execution_scopes (id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at) VALUES ('round', 'run', 'loop', 1, 1, '{\"carried\":{\"draft\":\"text\"}}', 3, 3)", []).unwrap();
        connection.execute("INSERT INTO workflow_node_runs (id, run_id, scope_id, node_id, node_type, session_id, status, input, output, created_at, updated_at) VALUES ('writer-1', 'run', 'round', 'writer', 'agent', 'session', 1, 'draft', 'partial', 3, 3)", []).unwrap();
        reconcile_database_versions(&mut connection, &previous, &TestClock::new(4)).unwrap();
        let record: String = connection
            .query_row(
                "SELECT record FROM workflow_scope_downgrade_archive WHERE scope_id = 'round'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let archive: serde_json::Value = serde_json::from_str(&record).unwrap();
        assert_eq!(
            archive["nodes"][0],
            serde_json::json!({
                "id":"writer-1", "runId":"run", "nodeId":"writer", "nodeType":"agent",
                "sessionId":"session", "status":1, "input":"draft", "output":"partial",
                "error":null, "payload":null, "startedAt":null, "finishedAt":null,
                "createdAt":3, "updatedAt":3, "isDeleted":0
            })
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT run_status FROM workflow_runs WHERE id = 'run'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status, is_deleted FROM workflow_node_runs WHERE id = 'writer-1'",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                )
                .unwrap(),
            (4, 1)
        );
        for _ in 0..2 {
            reconcile_database_versions(&mut connection, &target, &TestClock::new(5)).unwrap();
            reconcile_database_versions(&mut connection, &previous, &TestClock::new(6)).unwrap();
            assert_eq!(
                connection
                    .query_row(
                        "SELECT record FROM workflow_scope_downgrade_archive WHERE scope_id = ?1",
                        params!["round"],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                record
            );
        }
    });
}

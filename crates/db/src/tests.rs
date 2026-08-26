use std::path::Path;

use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::{
    AppliedMigration, DatabaseBootstrapper, DatabaseError, DatabaseLocation, Migration,
    MigrationCatalog, MigrationDirection, TimestampSource, default_migration_catalog,
};

/// Supplies a deterministic migration timestamp without mutating process-wide clock state.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource {
    now: i64,
}

impl TimestampSource for FixedTimestampSource {
    /// Returns the fixed timestamp selected by the schema test fixture.
    fn current_timestamp_millis(&self) -> i64 {
        self.now
    }
}

/// Verifies the current schema includes the linear Effect persistence migration.
#[test]
fn bootstraps_the_current_workspace_schema() {
    let catalog = default_migration_catalog().expect("build migration catalog");
    assert_eq!(
        catalog.target_versions(),
        ["0001", "0002", "0003", "0004", "0005", "0006"]
    );

    let database = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource {
            now: 1_700_000_000_000,
        })
        .bootstrap(&DatabaseLocation::in_memory(), &catalog)
        .expect("bootstrap database")
    });

    assert_eq!(
        load_table_names(database.connection()),
        vec![
            "agents",
            "effect_audit_events",
            "effect_conditions",
            "effect_consumer_status",
            "effect_managed_items",
            "effect_operation_artifacts",
            "effect_operations",
            "effect_propagation_requests",
            "effect_reconcile_requests",
            "effect_source_heads",
            "effect_source_revisions",
            "effect_sources",
            "effect_surface_consumers",
            "effect_surface_status",
            "effect_surfaces",
            "git_cleanup_jobs",
            "migrations",
            "plugin_marketplace_source",
            "projects",
            "sessions",
            "skills",
            "tasks",
            "user_config",
            "workflow_node_runs",
            "workflow_runs",
            "workflow_snapshots",
            "workflows",
            "workspace_effect_desired_items",
            "workspace_effects",
            "workspace_locations",
            "workspace_provisioning",
            "workspaces",
            "worktree_provisioning_leases",
            "worktrees",
        ],
    );
    assert_eq!(
        load_applied_migrations(database.connection()),
        expected_applied_migrations(&catalog, 1_700_000_000_000),
    );
}

/// Verifies runtime ownership columns point directly at workspaces and no longer encode task-run variants.
#[test]
fn runtime_tables_use_direct_workspace_ownership() {
    let database = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource { now: 1 })
            .bootstrap(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap database")
    });

    assert_eq!(
        load_table_column_names(database.connection(), "sessions"),
        vec![
            "id",
            "workspace_id",
            "title",
            "agent_cli",
            "agent_session_id",
            "history_degraded_reason",
            "status",
            "created_at",
            "updated_at",
            "is_deleted",
        ],
    );
    assert_eq!(
        load_table_column_names(database.connection(), "workflow_runs"),
        vec![
            "id",
            "workspace_id",
            "workflow_id",
            "snapshot_id",
            "name",
            "run_status",
            "state",
            "input",
            "output",
            "error",
            "payload",
            "started_at",
            "finished_at",
            "created_at",
            "updated_at",
            "is_deleted",
        ],
    );
    assert_eq!(
        load_table_column_names(database.connection(), "tasks"),
        vec![
            "id",
            "workspace_id",
            "title",
            "created_at",
            "updated_at",
            "is_deleted",
        ],
    );
    assert_eq!(
        load_table_column_names(database.connection(), "worktrees"),
        vec![
            "workspace_id",
            "branch_name",
            "base_commit_id",
            "created_at",
            "updated_at",
            "is_deleted",
        ],
    );
    assert_eq!(
        database
            .connection()
            .query_row(
                "SELECT pk FROM pragma_table_info('worktrees') WHERE name = 'workspace_id'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("load worktree workspace primary-key position"),
        1,
    );
}

/// Verifies a database with a shorter valid prefix receives and snapshots only the missing tail.
#[test]
fn applies_missing_migrations_in_order() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("missing-tail.sqlite3");
    let migrations = test_migrations();
    let prefix = MigrationCatalog::with_target_versions(migrations.clone(), vec!["0001"])
        .expect("build prefix catalog");
    let full = MigrationCatalog::new(migrations).expect("build full catalog");

    bootstrap_file_database(&database_path, &prefix, 100).expect("apply prefix");
    bootstrap_file_database(&database_path, &full, 200).expect("apply missing tail");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(
        load_applied_migrations(&connection),
        vec![
            applied_from(&full, "0001", 100),
            applied_from(&full, "0002", 200),
            applied_from(&full, "0003", 200),
        ]
    );
}

/// Verifies matching SQL snapshots make reconciliation a no-op regardless of the new clock value.
#[test]
fn unchanged_migration_content_is_a_no_op() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("no-op.sqlite3");
    let catalog =
        MigrationCatalog::new(vec![table_migration("0001", "alpha")]).expect("build catalog");

    bootstrap_file_database(&database_path, &catalog, 100).expect("apply catalog");
    bootstrap_file_database(&database_path, &catalog, 200).expect("reconcile unchanged catalog");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(
        load_applied_migrations(&connection),
        vec![applied_from(&catalog, "0001", 100)]
    );
}

/// Verifies execution timestamps are metadata and do not participate in SQL drift detection.
#[test]
fn differing_execution_time_does_not_rebuild_schema() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("timestamp.sqlite3");
    let catalog =
        MigrationCatalog::new(vec![table_migration("0001", "alpha")]).expect("build catalog");

    bootstrap_file_database(&database_path, &catalog, 100).expect("apply catalog");
    let connection = Connection::open(&database_path).expect("open database");
    connection
        .execute(
            "UPDATE migrations SET executed_at = 777 WHERE version = '0001'",
            [],
        )
        .expect("change timestamp metadata");
    drop(connection);
    bootstrap_file_database(&database_path, &catalog, 200).expect("reconcile catalog");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(
        load_applied_migrations(&connection),
        vec![applied_from(&catalog, "0001", 777)]
    );
}

/// Verifies changing only rollback SQL still rebuilds and refreshes both snapshots.
#[test]
fn changed_down_sql_rebuilds_the_migration() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("down-rewrite.sqlite3");
    let old_catalog = MigrationCatalog::new(vec![sql_migration(
        "0001",
        "CREATE TABLE alpha (id INTEGER PRIMARY KEY);",
        "DROP TABLE alpha;",
    )])
    .expect("build old catalog");
    let rewritten_catalog = MigrationCatalog::new(vec![sql_migration(
        "0001",
        "CREATE TABLE alpha (id INTEGER PRIMARY KEY);",
        "DROP TABLE IF EXISTS alpha;",
    )])
    .expect("build rewritten catalog");

    bootstrap_file_database(&database_path, &old_catalog, 100).expect("apply old catalog");
    bootstrap_file_database(&database_path, &rewritten_catalog, 200)
        .expect("rebuild changed rollback");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(table_exists(&connection, "alpha"), true);
    assert_eq!(
        load_applied_migrations(&connection),
        vec![applied_from(&rewritten_catalog, "0001", 200)]
    );
}

/// Verifies rewriting the latest migration uses its persisted rollback before applying new SQL.
#[test]
fn rebuilds_changed_latest_migration_with_persisted_down_sql() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("latest-rewrite.sqlite3");
    let old_catalog = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        sql_migration(
            "0002",
            "CREATE TABLE old_beta (id INTEGER PRIMARY KEY);",
            "DROP TABLE old_beta;",
        ),
    ])
    .expect("build old catalog");
    let rewritten_catalog = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        sql_migration(
            "0002",
            "CREATE TABLE new_beta (id INTEGER PRIMARY KEY);",
            "THIS CURRENT DOWN SQL MUST NOT RUN;",
        ),
    ])
    .expect("build rewritten catalog");

    bootstrap_file_database(&database_path, &old_catalog, 100).expect("apply old catalog");
    bootstrap_file_database(&database_path, &rewritten_catalog, 200)
        .expect("rebuild latest migration");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(table_exists(&connection, "old_beta"), false);
    assert_eq!(table_exists(&connection, "new_beta"), true);
    assert_eq!(
        load_applied_migrations(&connection),
        vec![
            applied_from(&rewritten_catalog, "0001", 100),
            applied_from(&rewritten_catalog, "0002", 200),
        ]
    );
}

/// Verifies drift in an earlier migration rebuilds every applied migration after that position.
#[test]
fn rebuilds_the_entire_suffix_after_earlier_sql_changes() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("suffix-rewrite.sqlite3");
    let old_catalog = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        table_migration("0002", "old_beta"),
        table_migration("0003", "gamma"),
    ])
    .expect("build old catalog");
    let rewritten_catalog = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        table_migration("0002", "new_beta"),
        table_migration("0003", "gamma"),
    ])
    .expect("build rewritten catalog");

    bootstrap_file_database(&database_path, &old_catalog, 100).expect("apply old catalog");
    bootstrap_file_database(&database_path, &rewritten_catalog, 200).expect("rebuild suffix");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(table_exists(&connection, "old_beta"), false);
    assert_eq!(table_exists(&connection, "new_beta"), true);
    assert_eq!(table_exists(&connection, "gamma"), true);
    assert_eq!(
        load_applied_migrations(&connection),
        vec![
            applied_from(&rewritten_catalog, "0001", 100),
            applied_from(&rewritten_catalog, "0002", 200),
            applied_from(&rewritten_catalog, "0003", 200),
        ]
    );
}

/// Verifies a failing persisted rollback keeps both its schema changes and bookkeeping row.
#[test]
fn failed_down_transaction_preserves_the_applied_migration() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("failed-down.sqlite3");
    let migrations = vec![
        table_migration("0001", "alpha"),
        sql_migration(
            "0002",
            "CREATE TABLE beta (id INTEGER PRIMARY KEY);",
            "DROP TABLE beta; THIS IS NOT SQL;",
        ),
    ];
    let full = MigrationCatalog::new(migrations.clone()).expect("build full catalog");
    let prefix = MigrationCatalog::with_target_versions(migrations, vec!["0001"])
        .expect("build prefix catalog");

    bootstrap_file_database(&database_path, &full, 100).expect("apply full catalog");
    let error =
        bootstrap_file_database(&database_path, &prefix, 200).expect_err("rollback must fail");

    assert_migration_step_failed(&error, "0002", MigrationDirection::Down);
    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(table_exists(&connection, "beta"), true);
    assert_eq!(load_applied_migrations(&connection).len(), 2);
}

/// Verifies a failed rewritten up leaves the database at the successfully rolled-back prefix.
#[test]
fn failed_rebuild_up_stays_at_the_rolled_back_state() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("failed-rebuild-up.sqlite3");
    let old_catalog = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        table_migration("0002", "old_beta"),
    ])
    .expect("build old catalog");
    let rewritten_catalog = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        sql_migration(
            "0002",
            "CREATE TABLE new_beta (id INTEGER PRIMARY KEY); THIS IS NOT SQL;",
            "DROP TABLE new_beta;",
        ),
    ])
    .expect("build rewritten catalog");

    bootstrap_file_database(&database_path, &old_catalog, 100).expect("apply old catalog");
    let error = bootstrap_file_database(&database_path, &rewritten_catalog, 200)
        .expect_err("rewritten up must fail");

    assert_migration_step_failed(&error, "0002", MigrationDirection::Up);
    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(table_exists(&connection, "old_beta"), false);
    assert_eq!(table_exists(&connection, "new_beta"), false);
    assert_eq!(
        load_applied_migrations(&connection),
        vec![applied_from(&rewritten_catalog, "0001", 100)]
    );
}

/// Verifies shortening the target performs the ordinary reverse-order tail rollback.
#[test]
fn shorter_target_rolls_back_the_applied_tail() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("short-target.sqlite3");
    let migrations = test_migrations();
    let full = MigrationCatalog::new(migrations.clone()).expect("build full catalog");
    let prefix = MigrationCatalog::with_target_versions(migrations, vec!["0001"])
        .expect("build prefix catalog");

    bootstrap_file_database(&database_path, &full, 100).expect("apply full catalog");
    bootstrap_file_database(&database_path, &prefix, 200).expect("roll back tail");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(table_exists(&connection, "alpha"), true);
    assert_eq!(table_exists(&connection, "beta"), false);
    assert_eq!(table_exists(&connection, "gamma"), false);
    assert_eq!(
        load_applied_migrations(&connection),
        vec![applied_from(&full, "0001", 100)]
    );
}

/// Verifies known versions in an illegal position and versions absent from the catalog stay errors.
#[test]
fn rejects_reordered_and_unknown_applied_versions() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let diverged_path = temp_dir.path().join("diverged.sqlite3");
    let diverged = MigrationCatalog::new(vec![
        table_migration("0001", "alpha"),
        table_migration("0003", "gamma"),
    ])
    .expect("build diverged catalog");
    let expected = MigrationCatalog::new(test_migrations()).expect("build expected catalog");
    bootstrap_file_database(&diverged_path, &diverged, 100).expect("apply diverged catalog");

    let error = bootstrap_file_database(&diverged_path, &expected, 200)
        .expect_err("reordered history must fail");
    assert_eq!(
        match error {
            DatabaseError::DivergedMigrationHistory {
                position,
                expected,
                found,
            } => Some((position, expected, found)),
            _ => None,
        },
        Some((1, "0002".to_string(), "0003".to_string()))
    );

    let unknown_path = temp_dir.path().join("unknown.sqlite3");
    bootstrap_file_database(&unknown_path, &expected, 100).expect("apply expected catalog");
    let connection = Connection::open(&unknown_path).expect("open database");
    connection
        .execute(
            "INSERT INTO migrations (version, up_sql, down_sql, executed_at) VALUES ('9999', '', '', 100)",
            [],
        )
        .expect("insert unknown migration");
    drop(connection);

    let error = bootstrap_file_database(&unknown_path, &expected, 200)
        .expect_err("unknown history must fail");
    assert_eq!(
        match error {
            DatabaseError::UnknownAppliedMigrationVersion { version } => Some(version),
            _ => None,
        },
        Some("9999".to_string())
    );
}

/// Verifies multiple changed migrations roll back in reverse and reapply in forward order.
#[test]
fn rebuilds_multiple_changed_migrations_in_directional_order() {
    let temp_dir = TempDir::new().expect("create temporary directory");
    let database_path = temp_dir.path().join("ordered-rebuild.sqlite3");
    let foundation = sql_migration(
        "0001",
        "CREATE TABLE events (sequence INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL);",
        "DROP TABLE events;",
    );
    let old_catalog = MigrationCatalog::new(vec![
        foundation.clone(),
        sql_migration(
            "0002",
            "INSERT INTO events (name) VALUES ('old-2');",
            "DELETE FROM events WHERE name = 'old-2'; INSERT INTO events (name) VALUES ('down-2');",
        ),
        sql_migration(
            "0003",
            "INSERT INTO events (name) VALUES ('old-3');",
            "DELETE FROM events WHERE name = 'old-3'; INSERT INTO events (name) VALUES ('down-3');",
        ),
    ])
    .expect("build old catalog");
    let rewritten_catalog = MigrationCatalog::new(vec![
        foundation,
        sql_migration(
            "0002",
            "INSERT INTO events (name) VALUES ('up-2-new');",
            "DELETE FROM events WHERE name = 'up-2-new';",
        ),
        sql_migration(
            "0003",
            "INSERT INTO events (name) VALUES ('up-3-new');",
            "DELETE FROM events WHERE name = 'up-3-new';",
        ),
    ])
    .expect("build rewritten catalog");

    bootstrap_file_database(&database_path, &old_catalog, 100).expect("apply old catalog");
    bootstrap_file_database(&database_path, &rewritten_catalog, 200)
        .expect("rebuild changed suffix");

    let connection = Connection::open(&database_path).expect("open database");
    assert_eq!(
        load_text_column(&connection, "SELECT name FROM events ORDER BY sequence"),
        vec!["down-3", "down-2", "up-2-new", "up-3-new"]
    );
}

/// Reads table names in the same deterministic order used by the schema assertion.
fn load_table_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare table query");
    statement
        .query_map([], |row| row.get(0))
        .expect("query table names")
        .collect::<Result<Vec<_>, _>>()
        .expect("read table names")
}

/// Reads a table's declared columns without coupling the test to SQLite's SQL text formatting.
fn load_table_column_names(connection: &Connection, table_name: &str) -> Vec<String> {
    let statement = format!("PRAGMA table_info({table_name})");
    connection
        .prepare(&statement)
        .expect("prepare column query")
        .query_map([], |row| row.get(1))
        .expect("query column names")
        .collect::<Result<Vec<_>, _>>()
        .expect("read column names")
}

/// Loads migration bookkeeping rows for exact bootstrap assertions.
fn load_applied_migrations(connection: &Connection) -> Vec<AppliedMigration> {
    let mut statement = connection
        .prepare("SELECT version, up_sql, down_sql, executed_at FROM migrations ORDER BY version")
        .expect("prepare migration query");
    statement
        .query_map([], |row| {
            Ok(AppliedMigration::new(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .expect("query migrations")
        .collect::<Result<Vec<_>, _>>()
        .expect("read migrations")
}

/// Builds the exact rows expected when every target migration is applied at one timestamp.
fn expected_applied_migrations(
    catalog: &MigrationCatalog,
    executed_at: i64,
) -> Vec<AppliedMigration> {
    catalog
        .target_versions()
        .iter()
        .map(|version| applied_from(catalog, version, executed_at))
        .collect()
}

/// Builds an expected applied row from the current catalog SQL snapshot.
fn applied_from(catalog: &MigrationCatalog, version: &str, executed_at: i64) -> AppliedMigration {
    let migration = catalog.migration(version).expect("find migration");
    AppliedMigration::new(
        migration.version(),
        migration.up_sql(),
        migration.down_sql(),
        executed_at,
    )
}

/// Reconciles a file database while keeping logging scoped to this test operation.
fn bootstrap_file_database(
    path: &Path,
    catalog: &MigrationCatalog,
    now: i64,
) -> Result<(), DatabaseError> {
    with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource { now })
            .bootstrap(&DatabaseLocation::path(path), catalog)
            .map(|_| ())
    })
}

/// Builds a reusable three-migration table catalog for prefix and ordering tests.
fn test_migrations() -> Vec<Migration> {
    vec![
        table_migration("0001", "alpha"),
        table_migration("0002", "beta"),
        table_migration("0003", "gamma"),
    ]
}

/// Builds a migration from explicit SQL used to exercise rewrite and failure semantics.
fn sql_migration(version: &'static str, up_sql: &'static str, down_sql: &'static str) -> Migration {
    let up_statements = Box::leak(vec![up_sql].into_boxed_slice());
    let down_statements = Box::leak(vec![down_sql].into_boxed_slice());
    Migration::new(version, up_statements, down_statements)
}

/// Builds a reversible single-table migration with leaked test-only SQL storage.
fn table_migration(version: &'static str, table_name: &'static str) -> Migration {
    let up_sql =
        Box::leak(format!("CREATE TABLE {table_name} (id INTEGER PRIMARY KEY);").into_boxed_str());
    let down_sql = Box::leak(format!("DROP TABLE {table_name};").into_boxed_str());
    sql_migration(version, up_sql, down_sql)
}

/// Reports whether a named table exists in the current SQLite schema.
fn table_exists(connection: &Connection, table_name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table_name],
            |row| row.get::<_, bool>(0),
        )
        .expect("query table existence")
}

/// Loads a single text column for exact ordering assertions.
fn load_text_column(connection: &Connection, sql: &str) -> Vec<String> {
    connection
        .prepare(sql)
        .expect("prepare text query")
        .query_map([], |row| row.get(0))
        .expect("query text values")
        .collect::<Result<Vec<_>, _>>()
        .expect("read text values")
}

/// Verifies a migration failure reports the expected version and direction.
fn assert_migration_step_failed(
    error: &DatabaseError,
    expected_version: &str,
    expected_direction: MigrationDirection,
) {
    assert_eq!(
        match error {
            DatabaseError::MigrationStepFailed {
                version, direction, ..
            } => Some((version.as_str(), *direction)),
            _ => None,
        },
        Some((expected_version, expected_direction))
    );
}

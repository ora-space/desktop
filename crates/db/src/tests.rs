use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ora_logging::{with_recorded_trace_logging, with_trace_logging};
use pretty_assertions::assert_eq;
use rusqlite::{Connection, ErrorCode, params};
use tempfile::TempDir;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use crate::{
    AppliedMigration, DatabaseBootstrapper, DatabaseError, DatabaseLocation, Migration,
    MigrationCatalog, MigrationDirection, TimestampSource, default_migration_catalog,
};

/// Produces deterministic timestamps so migration bookkeeping tests can assert full records.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource {
    now: i64,
}

impl TimestampSource for FixedTimestampSource {
    /// Returns the preconfigured timestamp for every migration applied in a test step.
    fn current_timestamp_millis(&self) -> i64 {
        self.now
    }
}

/// Verifies a fresh database bootstrap applies the shipped schema migration and records its timestamp.
#[test]
fn bootstraps_empty_database_with_default_catalog() {
    let catalog = default_migration_catalog().unwrap();
    let database = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource {
            now: 1_700_000_000_000,
        })
        .bootstrap(&DatabaseLocation::in_memory(), &catalog)
        .unwrap()
    });

    assert_eq!(
        load_table_names(database.connection()),
        vec![
            "agents".to_string(),
            "artifacts".to_string(),
            "migrations".to_string(),
            "project_work_contexts".to_string(),
            "projects".to_string(),
            "sessions".to_string(),
            "skills".to_string(),
            "task_diff_comments".to_string(),
            "tasks".to_string(),
            "virtual_entries".to_string(),
            "virtual_folders".to_string(),
            "worktrees".to_string(),
        ]
    );
    assert_eq!(
        load_applied_migrations(database.connection()),
        vec![
            AppliedMigration::new("0001", 1_700_000_000_000),
            AppliedMigration::new("0002", 1_700_000_000_000),
            AppliedMigration::new("0003", 1_700_000_000_000),
            AppliedMigration::new("0004", 1_700_000_000_000),
        ]
    );
}

/// Verifies the catalog creates ID-keyed schema without name indexes and removes it during rollback.
#[test]
fn manages_skill_and_agent_definition_schema_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("skill-agent.sqlite3");
    let catalog = default_migration_catalog().unwrap();
    let migrations = ["0001", "0002", "0003", "0004"].map(|version| {
        catalog
            .migration(version)
            .cloned()
            .unwrap_or_else(|| panic!("missing migration {version}"))
    });

    bootstrap_file_database(&database_path, catalog, 1_700_000_000_000);

    let connection = Connection::open(&database_path).unwrap();
    for table_name in ["skills", "agents"] {
        assert_eq!(
            load_table_column_names(&connection, table_name),
            vec![
                "id".to_string(),
                "name".to_string(),
                "description".to_string(),
                "created_at".to_string(),
                "updated_at".to_string(),
                "is_deleted".to_string(),
            ]
        );

        connection
            .execute(
                &format!(
                    "INSERT INTO {table_name} (id, name, description, created_at, updated_at, is_deleted)\n                     VALUES (?1, ?2, ?3, ?4, ?5, 0)"
                ),
                params!["first", "opencode", "OpenCode", 1_i64, 1_i64],
            )
            .unwrap();
        connection
            .execute(
                &format!(
                    "INSERT INTO {table_name} (id, name, description, created_at, updated_at, is_deleted)\n                     VALUES (?1, ?2, ?3, ?4, ?5, 0)"
                ),
                params!["second", "opencode", "OpenCode duplicate", 2_i64, 2_i64],
            )
            .unwrap();
        let duplicate_id = connection.execute(
            &format!(
                "INSERT INTO {table_name} (id, name, description, created_at, updated_at, is_deleted)\n                 VALUES (?1, ?2, ?3, ?4, ?5, 0)"
            ),
            params!["first", "different-name", "Duplicate ID", 3_i64, 3_i64],
        );

        assert_eq!(
            matches!(
                duplicate_id,
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == ErrorCode::ConstraintViolation
            ),
            true
        );

        let deleted_rows = connection
            .execute(
                &format!("UPDATE {table_name} SET is_deleted = 1 WHERE id = ?1"),
                params!["first"],
            )
            .unwrap();
        assert_eq!(deleted_rows, 1);
        connection
            .execute(
                &format!(
                    "INSERT INTO {table_name} (id, name, description, created_at, updated_at, is_deleted)\n                 VALUES (?1, ?2, ?3, ?4, ?5, 0)"
                ),
                params!["third", "opencode", "OpenCode replacement", 4_i64, 4_i64],
            )
            .unwrap();
    }

    drop(connection);
    let rollback_catalog =
        MigrationCatalog::with_target_versions(migrations.to_vec(), vec!["0001", "0002"]).unwrap();
    bootstrap_file_database(&database_path, rollback_catalog, 1_700_000_000_100);

    let connection = Connection::open(&database_path).unwrap();
    assert_eq!(table_exists(&connection, "skills"), false);
    assert_eq!(table_exists(&connection, "agents"), false);
}

/// Verifies the runner applies only the missing tail of a linear migration history in ascending order.
#[test]
fn applies_missing_migrations_in_ascending_order() {
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("upgrade.sqlite3");

    bootstrap_file_database(
        &database_path,
        test_catalog_with_target_prefix(1).unwrap(),
        100,
    );
    bootstrap_file_database(&database_path, test_catalog().unwrap(), 200);

    let connection = Connection::open(&database_path).unwrap();

    assert_eq!(
        load_applied_migrations(&connection),
        vec![
            AppliedMigration::new("0001", 100),
            AppliedMigration::new("0002", 200),
            AppliedMigration::new("0003", 200),
        ]
    );
    assert_eq!(table_exists(&connection, "beta"), true);
    assert_eq!(table_exists(&connection, "gamma"), true);
}

/// Verifies the runner rolls back extra targeted versions in descending order while preserving older records.
#[test]
fn rolls_back_extra_versions_in_descending_order() {
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("rollback.sqlite3");

    bootstrap_file_database(&database_path, test_catalog().unwrap(), 300);
    bootstrap_file_database(
        &database_path,
        test_catalog_with_target_prefix(2).unwrap(),
        400,
    );

    let connection = Connection::open(&database_path).unwrap();

    assert_eq!(
        load_applied_migrations(&connection),
        vec![
            AppliedMigration::new("0001", 300),
            AppliedMigration::new("0002", 300),
        ]
    );
    assert_eq!(table_exists(&connection, "beta"), true);
    assert_eq!(table_exists(&connection, "gamma"), false);
}

/// Verifies a mismatch inside the shared prefix fails fast instead of guessing at repair steps.
#[test]
fn rejects_diverged_history_in_shared_prefix() {
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("diverged.sqlite3");

    bootstrap_file_database(&database_path, diverged_catalog().unwrap(), 500);

    let error = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource { now: 600 })
            .bootstrap(
                &DatabaseLocation::path(&database_path),
                &test_catalog().unwrap(),
            )
            .unwrap_err()
    });

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
}

/// Verifies a failing up step does not record the version whose SQL could not be installed.
#[test]
fn leaves_failed_up_migration_unrecorded() {
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("failed-up.sqlite3");

    bootstrap_file_database(
        &database_path,
        MigrationCatalog::new(vec![create_table_migration("0001", "alpha")]).unwrap(),
        700,
    );

    let error = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource { now: 800 })
            .bootstrap(
                &DatabaseLocation::path(&database_path),
                &MigrationCatalog::new(vec![
                    create_table_migration("0001", "alpha"),
                    broken_up_migration("0002"),
                ])
                .unwrap(),
            )
            .unwrap_err()
    });

    assert_migration_step_failed(&error, "0002", MigrationDirection::Up);

    let connection = Connection::open(&database_path).unwrap();

    assert_eq!(
        load_applied_migrations(&connection),
        vec![AppliedMigration::new("0001", 700)]
    );
}

/// Verifies a failing down step keeps the extra version recorded because the rollback never commits.
#[test]
fn leaves_failed_down_migration_recorded() {
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("failed-down.sqlite3");

    bootstrap_file_database(
        &database_path,
        MigrationCatalog::new(vec![
            create_table_migration("0001", "alpha"),
            broken_down_migration("0002"),
        ])
        .unwrap(),
        800,
    );

    let error = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource { now: 900 })
            .bootstrap(
                &DatabaseLocation::path(&database_path),
                &MigrationCatalog::with_target_versions(
                    vec![
                        create_table_migration("0001", "alpha"),
                        broken_down_migration("0002"),
                    ],
                    vec!["0001"],
                )
                .unwrap(),
            )
            .unwrap_err()
    });

    assert_migration_step_failed(&error, "0002", MigrationDirection::Down);

    let connection = Connection::open(&database_path).unwrap();

    assert_eq!(
        load_applied_migrations(&connection),
        vec![
            AppliedMigration::new("0001", 800),
            AppliedMigration::new("0002", 800),
        ]
    );
}

/// Verifies bootstrap and migration reconciliation emit structured success and failure events.
#[test]
fn emits_structured_bootstrap_and_migration_events() {
    let success_recorder = EventRecorder::default();
    with_recorded_trace_logging(success_recorder.layer(), || {
        DatabaseBootstrapper::new(FixedTimestampSource { now: 42 })
            .bootstrap(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
    });

    assert_eq!(
        success_recorder.events().into_iter().any(|event| {
            event_has_fields(
                &event,
                &[
                    ("method", "bootstrap"),
                    ("message", "database bootstrap complete"),
                    ("operation", "database_bootstrap"),
                ],
            )
        }),
        true
    );

    let failure_recorder = EventRecorder::default();
    let temp_dir = TempDir::new().unwrap();
    let database_path = temp_dir.path().join("failed-up.sqlite3");

    with_recorded_trace_logging(failure_recorder.layer(), || {
        bootstrap_file_database(
            &database_path,
            MigrationCatalog::new(vec![create_table_migration("0001", "alpha")]).unwrap(),
            700,
        );
        let error = DatabaseBootstrapper::new(FixedTimestampSource { now: 800 }).bootstrap(
            &DatabaseLocation::path(&database_path),
            &MigrationCatalog::new(vec![
                create_table_migration("0001", "alpha"),
                broken_up_migration("0002"),
            ])
            .unwrap(),
        );

        assert_eq!(
            matches!(
                error,
                Err(DatabaseError::MigrationStepFailed {
                    version,
                    direction: MigrationDirection::Up,
                    ..
                }) if version == "0002"
            ),
            true
        );
    });

    assert_eq!(
        failure_recorder.events().into_iter().any(|event| {
            event_has_fields(
                &event,
                &[
                    ("direction", "up"),
                    ("error.kind", "migration_step_failed"),
                    ("message", "migration step failed"),
                    ("method", "execute_migration_step"),
                    ("migration_version", "0002"),
                    ("operation", "migration_execute"),
                ],
            )
        }),
        true
    );
}

/// Opens a file-backed database through the bootstrapper and drops the handle once reconciliation finishes.
fn bootstrap_file_database(path: &Path, catalog: MigrationCatalog, now: i64) {
    with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource { now })
            .bootstrap(&DatabaseLocation::path(path), &catalog)
            .unwrap();
    });
}

/// Loads visible user tables in alphabetical order so schema assertions remain stable across SQLite versions.
fn load_table_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name ASC",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap();

    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

/// Loads persisted migration rows in the same order used by the reconciliation algorithm.
fn load_applied_migrations(connection: &Connection) -> Vec<AppliedMigration> {
    let mut statement = connection
        .prepare("SELECT version, executed_at FROM migrations ORDER BY version ASC")
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok(AppliedMigration::new(
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
            ))
        })
        .unwrap();

    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

/// Reports whether a table currently exists so upgrade and rollback tests can assert schema effects directly.
fn table_exists(connection: &Connection, table_name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table_name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

/// Loads a table's column names in declaration order so migration tests can assert its shape.
fn load_table_column_names(connection: &Connection, table_name: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap();

    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

/// Verifies a migration-step error identifies the version and direction while preserving the SQL parser context.
fn assert_migration_step_failed(
    error: &DatabaseError,
    expected_version: &str,
    expected_direction: MigrationDirection,
) {
    match error {
        DatabaseError::MigrationStepFailed {
            version,
            direction,
            source,
        } => {
            assert_eq!(version, expected_version);
            assert_eq!(direction, &expected_direction);
            assert_eq!(
                source.to_string().contains("near \"THIS\": syntax error"),
                true
            );
        }
        _ => panic!("expected migration step failure, got {error:?}"),
    }
}

/// Builds the reusable three-step catalog used by upgrade and rollback behavior tests.
fn test_catalog() -> Result<MigrationCatalog, DatabaseError> {
    MigrationCatalog::new(vec![
        create_table_migration("0001", "alpha"),
        create_table_migration("0002", "beta"),
        create_table_migration("0003", "gamma"),
    ])
}

/// Builds the same test catalog with a shorter active prefix to simulate a controlled rollback target.
fn test_catalog_with_target_prefix(
    prefix_length: usize,
) -> Result<MigrationCatalog, DatabaseError> {
    let migrations = vec![
        create_table_migration("0001", "alpha"),
        create_table_migration("0002", "beta"),
        create_table_migration("0003", "gamma"),
    ];
    let target_versions = migrations
        .iter()
        .take(prefix_length)
        .map(Migration::version)
        .collect();

    MigrationCatalog::with_target_versions(migrations, target_versions)
}

/// Builds an alternate catalog whose second version intentionally diverges from the main test sequence.
fn diverged_catalog() -> Result<MigrationCatalog, DatabaseError> {
    MigrationCatalog::new(vec![
        create_table_migration("0001", "alpha"),
        create_table_migration("0003", "gamma"),
    ])
}

/// Builds a simple migration that creates and drops one named table.
fn create_table_migration(version: &'static str, table_name: &'static str) -> Migration {
    let up_sql =
        Box::leak(format!("CREATE TABLE {table_name} (id INTEGER PRIMARY KEY);").into_boxed_str());
    let down_sql = Box::leak(format!("DROP TABLE IF EXISTS {table_name};").into_boxed_str());
    let up_statements = Box::leak(vec![up_sql as &'static str].into_boxed_slice());
    let down_statements = Box::leak(vec![down_sql as &'static str].into_boxed_slice());

    Migration::new(version, up_statements, down_statements)
}

/// Reports whether one recorded event includes all expected field/value pairs.
fn event_has_fields(event: &LoggedEvent, expected_fields: &[(&str, &str)]) -> bool {
    expected_fields.iter().all(|(field_name, expected_value)| {
        event.fields.get(*field_name).map(String::as_str) == Some(*expected_value)
    })
}

/// Captures one emitted event in a comparison-friendly structure for database logging assertions.
#[derive(Clone, Debug, Eq, PartialEq)]
struct LoggedEvent {
    level: String,
    target: String,
    fields: BTreeMap<String, String>,
}

/// Records tracing events into shared memory so database tests can assert structured outcomes.
#[derive(Clone, Debug, Default)]
struct EventRecorder {
    events: Arc<Mutex<Vec<LoggedEvent>>>,
}

impl EventRecorder {
    /// Builds the recording layer attached to one scoped test subscriber.
    fn layer(&self) -> RecordingLayer {
        RecordingLayer {
            events: self.events.clone(),
        }
    }

    /// Returns every captured event in emission order.
    fn events(&self) -> Vec<LoggedEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// Pushes each tracing event into the shared recorder without relying on global subscriber state.
#[derive(Clone, Debug)]
struct RecordingLayer {
    events: Arc<Mutex<Vec<LoggedEvent>>>,
}

impl<S> Layer<S> for RecordingLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    /// Converts each event into a stable, fully comparable structure for test assertions.
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(LoggedEvent {
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            fields: visitor.fields,
        });
    }
}

/// Records tracing fields as strings because these tests care about semantic content, not JSON formatting.
#[derive(Debug, Default)]
struct EventFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl tracing::field::Visit for EventFieldVisitor {
    /// Preserves string fields exactly as database logs emitted them.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Preserves signed integers in decimal form for stable assertions.
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Preserves unsigned integers in decimal form for stable assertions.
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Falls back to debug formatting for field types without a more specific visitor hook.
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.fields.insert(
            field.name().to_string(),
            format!("{value:?}").trim_matches('"').to_string(),
        );
    }
}

/// Builds a migration whose `up` SQL fails immediately so transaction rollback behavior can be asserted.
fn broken_up_migration(version: &'static str) -> Migration {
    Migration::new(
        version,
        &["THIS IS NOT VALID SQL"],
        &["DROP TABLE IF EXISTS broken_up;"],
    )
}

/// Builds a migration whose `down` SQL fails immediately so rollback bookkeeping can be asserted.
fn broken_down_migration(version: &'static str) -> Migration {
    Migration::new(
        version,
        &["CREATE TABLE broken_down (id INTEGER PRIMARY KEY);"],
        &["THIS IS NOT VALID SQL"],
    )
}

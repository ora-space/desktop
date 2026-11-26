use crate::migration::reconcile_database_versions;
use crate::{MigrationCatalog, default_migration_catalog, test_clock::TestClock};
use pretty_assertions::assert_eq;
use rusqlite::Connection;
use std::collections::BTreeMap;

/// Keeps the exact SQL definitions so rollback also detects changes to pre-existing objects.
fn schema_objects(connection: &Connection) -> BTreeMap<String, (String, Option<String>)> {
    connection
        .prepare("SELECT name, type, sql FROM sqlite_schema")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Exercises both directions of 0027 against the production catalog, even after later migrations land.
#[test]
fn migration_0027_round_trips_on_a_0026_database() {
    ora_logging::with_trace_logging(|| {
        let latest = default_migration_catalog().unwrap();
        let catalog_through = |version: &str| {
            MigrationCatalog::new(
                latest
                    .target_versions()
                    .iter()
                    .copied()
                    .take_while(|v| *v <= version)
                    .map(|v| latest.migration(v).unwrap().clone())
                    .collect(),
            )
            .unwrap()
        };
        let previous = catalog_through("0026");
        let current = catalog_through("0027");
        let mut connection = Connection::open_in_memory().unwrap();
        let migrate = |connection: &mut Connection, catalog: &MigrationCatalog, expected: &str| {
            reconcile_database_versions(connection, catalog, &TestClock::new(1)).unwrap();
            let version: String = connection
                .query_row("SELECT MAX(version) FROM migrations", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, expected);
        };
        migrate(&mut connection, &previous, "0026");
        let before = schema_objects(&connection);
        migrate(&mut connection, &current, "0027");
        let after = schema_objects(&connection);
        let mut preserved = after.clone();
        for (name, kind) in [
            ("workflow_debug_boot_protected", "view"),
            ("workflow_debug_dispatch", "table"),
            ("sqlite_autoindex_workflow_debug_dispatch_1", "index"),
            ("workflow_debug_end_dispatch", "trigger"),
        ] {
            assert_eq!(
                preserved.remove(name).map(|object| object.0).as_deref(),
                Some(kind),
                "{name}"
            );
        }
        assert_eq!(preserved, before);
        migrate(&mut connection, &previous, "0026");
        assert_eq!(schema_objects(&connection), before);
        migrate(&mut connection, &current, "0027");
        assert_eq!(schema_objects(&connection), after);
    });
}

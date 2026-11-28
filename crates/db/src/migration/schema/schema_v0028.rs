use super::Migration;

/// Track which operation retired each round and which slots still await fresh feedback.
pub fn migration() -> Migration {
    Migration::new(
        "0028",
        &[r#"
        CREATE TABLE workflow_debug_round_history (
            scope_id TEXT NOT NULL REFERENCES workflow_execution_scopes(id),
            operation_id INTEGER NOT NULL REFERENCES workflow_debug_operations(id) ON DELETE CASCADE,
            awaiting_replay INTEGER NOT NULL CHECK(awaiting_replay IN (0,1)),
            PRIMARY KEY(scope_id,operation_id)
        );
        CREATE UNIQUE INDEX workflow_debug_one_retired_slot ON workflow_debug_round_history(scope_id) WHERE awaiting_replay=1;
        INSERT INTO workflow_debug_round_history(scope_id, operation_id, awaiting_replay)
        SELECT s.id, o.id, 1 FROM workflow_execution_scopes s
        JOIN workflow_debug_operations o ON o.id = json_extract(s.state, '$.debugAwaitingReplay')
        WHERE o.run_id = s.run_id;
        UPDATE workflow_execution_scopes SET state = json_remove(state, '$.debugAwaitingReplay')
        WHERE json_type(state, '$.debugAwaitingReplay') IS NOT NULL;
    "#],
        &[r#"
        UPDATE workflow_execution_scopes AS s
        SET state = json_set(s.state, '$.debugAwaitingReplay', h.operation_id)
        FROM workflow_debug_round_history h WHERE h.scope_id = s.id AND h.awaiting_replay = 1;
        DROP TABLE workflow_debug_round_history;
    "#],
    )
}

#[cfg(test)]
mod tests {
    use crate::migration::{MigrationCatalog, reconcile_database_versions};
    use crate::test_clock::TestClock;
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;

    /// Reopening a coordinator database adds round history without replacing its dispatch state.
    #[test]
    fn upgrades_an_existing_coordinator_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ora.sqlite3");
        let catalog_through = |last_version: &str| {
            MigrationCatalog::new(
                super::super::migrations()
                    .into_iter()
                    .take_while(|migration| migration.version() <= last_version)
                    .collect(),
            )
            .unwrap()
        };
        {
            let mut connection = Connection::open(&path).unwrap();
            reconcile_database_versions(
                &mut connection,
                &catalog_through("0027"),
                &TestClock::new(1),
            )
            .unwrap();
            connection
                .prepare("SELECT run_id, operation_id FROM workflow_debug_dispatch")
                .unwrap();
        }
        let mut connection = Connection::open(&path).unwrap();
        reconcile_database_versions(
            &mut connection,
            &catalog_through("0028"),
            &TestClock::new(2),
        )
        .unwrap();
        let timestamps: (i64, i64) = connection
            .query_row(
                "SELECT a.executed_at, b.executed_at FROM migrations a, migrations b
                 WHERE a.version='0027' AND b.version='0028'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(timestamps, (1, 2));
        connection
            .prepare(
                "SELECT scope_id, operation_id, awaiting_replay FROM workflow_debug_round_history",
            )
            .unwrap();
    }
}

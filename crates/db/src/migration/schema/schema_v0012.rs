use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
ALTER TABLE workflow_node_runs ADD COLUMN iteration INTEGER NULL;
"#,
    r#"
CREATE INDEX idx_workflow_node_runs_iteration
    ON workflow_node_runs(run_id, iteration, created_at, id);
"#,
];

const DOWN_STATEMENTS: &[&str] = &[
    r#"
DROP INDEX IF EXISTS idx_workflow_node_runs_iteration;
"#,
    r#"
ALTER TABLE workflow_node_runs DROP COLUMN iteration;
"#,
];

/// Persists the composite-region round of each node-run row (ADR "iteration composite runtime" D2).
///
/// Outer rows keep `NULL`; rows executing inside an iteration region carry the round they
/// belong to (0-based, same value as `{iter}.index`). The same node may therefore hold one
/// row per round — the table's node-run-id primary key already accommodates that.
pub fn migration() -> Migration {
    Migration::new("0012", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use super::{DOWN_STATEMENTS, UP_STATEMENTS};
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;

    /// Legacy node-run rows upgrade with a NULL round and new rows can record rounds.
    #[test]
    fn adds_a_nullable_iteration_column_and_rolls_back() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE workflow_node_runs (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL,
                    node_id TEXT NOT NULL,
                    node_type TEXT NOT NULL,
                    status INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    is_deleted INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at)
                VALUES ('legacy', 'run-1', 'a', 'agent', 2, 1, 1);
                "#,
            )
            .unwrap();

        for statement in UP_STATEMENTS {
            connection.execute_batch(statement).unwrap();
        }
        let legacy_round: Option<i64> = connection
            .query_row(
                "SELECT iteration FROM workflow_node_runs WHERE id = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_round, None);

        connection
            .execute(
                "INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, iteration, created_at, updated_at)
                 VALUES ('round-1', 'run-1', 'fix', 'agent', 2, 1, 2, 2)",
                [],
            )
            .unwrap();
        let rounds: Vec<i64> = connection
            .prepare(
                "SELECT iteration FROM workflow_node_runs WHERE iteration IS NOT NULL ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .map(|round: Option<i64>| round.expect("round rows record their iteration"))
            .collect();
        assert_eq!(rounds, vec![1]);

        for statement in DOWN_STATEMENTS {
            connection.execute_batch(statement).unwrap();
        }
        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(workflow_node_runs)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "iteration"));
    }
}

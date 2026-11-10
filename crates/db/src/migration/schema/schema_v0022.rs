use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE workflow_document_imports (
    source_namespace TEXT NOT NULL,
    identifier       TEXT NOT NULL,
    document_path    TEXT NOT NULL,
    state            TEXT NOT NULL CHECK (state IN ('imported', 'conflict_skipped')),
    workflow_id      TEXT,
    recorded_at      INTEGER NOT NULL,
    PRIMARY KEY (source_namespace, identifier, document_path),
    CHECK ((state = 'imported') = (workflow_id IS NOT NULL))
);
"#];

const DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS workflow_document_imports;"];

/// Records which document paths a marketplace plugin already delivered a workflow for.
///
/// The table holds terminal outcomes only: a path either produced a copy, or collided with a
/// workflow of the same name the user already had. Both are permanent for that source path, which
/// is what makes an upgrade additive and a user deletion stay deleted. The record is deliberately
/// not a foreign key to `workflows`: it outlives the copy so re-importing a deleted workflow stays
/// refused, and `workflow_id` is informational only.
pub fn migration() -> Migration {
    Migration::new("0022", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::UP_STATEMENTS;

    #[test]
    fn keeps_one_terminal_state_per_source_path() {
        let connection = Connection::open_in_memory().expect("open database");
        for statement in UP_STATEMENTS {
            connection
                .execute_batch(statement)
                .expect("apply migration");
        }
        connection
            .execute(
                "INSERT INTO workflow_document_imports
                     (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                 VALUES ('official', 'ora.cdase-workflow', 'assets/workflows/1.0.0.json', 'imported', 'workflow-1', 1)",
                [],
            )
            .expect("insert imported record");
        connection
            .execute(
                "INSERT INTO workflow_document_imports
                     (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                 VALUES ('official', 'ora.cdase-workflow', 'assets/workflows/2.0.0.json', 'conflict_skipped', NULL, 1)",
                [],
            )
            .expect("insert conflict record");

        // The same source path can never carry a second outcome, which is what stops a reinstall
        // or upgrade from importing a path twice.
        assert!(
            connection
                .execute(
                    "INSERT INTO workflow_document_imports
                         (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                     VALUES ('official', 'ora.cdase-workflow', 'assets/workflows/1.0.0.json', 'imported', 'workflow-2', 2)",
                    [],
                )
                .is_err()
        );

        // The same file name under another namespace or plugin is a different source path.
        connection
            .execute(
                "INSERT INTO workflow_document_imports
                     (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                 VALUES ('other', 'ora.cdase-workflow', 'assets/workflows/1.0.0.json', 'imported', 'workflow-3', 2)",
                [],
            )
            .expect("insert same path under another namespace");
    }

    #[test]
    fn ties_the_recorded_workflow_to_an_imported_state() {
        let connection = Connection::open_in_memory().expect("open database");
        for statement in UP_STATEMENTS {
            connection
                .execute_batch(statement)
                .expect("apply migration");
        }
        assert!(
            connection
                .execute(
                    "INSERT INTO workflow_document_imports
                         (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                     VALUES ('official', 'ora.cdase-workflow', 'assets/workflows/1.0.0.json', 'imported', NULL, 1)",
                    [],
                )
                .is_err(),
            "an imported path must name the copy it created"
        );
        assert!(
            connection
                .execute(
                    "INSERT INTO workflow_document_imports
                         (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                     VALUES ('official', 'ora.cdase-workflow', 'assets/workflows/2.0.0.json', 'unknown', NULL, 1)",
                    [],
                )
                .is_err(),
            "only terminal states may be recorded"
        );
    }
}

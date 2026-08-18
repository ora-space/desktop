use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE tasks DROP COLUMN status;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE tasks ADD COLUMN status INTEGER NOT NULL DEFAULT 0;
"#];

/// Drops the unused kanban `tasks.status` column.
///
/// Task progress is session activity or workflow-run status; the integer
/// todo/doing/done field was only persisted and never drove behavior.
pub fn migration() -> Migration {
    Migration::new("0006", UP_STATEMENTS, DOWN_STATEMENTS)
}

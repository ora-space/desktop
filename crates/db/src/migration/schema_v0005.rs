use super::Migration;

// One nullable column rather than a flag paired with a reason: absence means the
// history is writable, and presence carries the explanation the user has to act
// on. There is no way to store "degraded, cause unknown".
const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions ADD COLUMN history_degraded_reason TEXT;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions DROP COLUMN history_degraded_reason;
"#];

/// Builds the migration that records why a session stopped writing its history.
pub fn migration() -> Migration {
    Migration::new("0005", UP_STATEMENTS, DOWN_STATEMENTS)
}

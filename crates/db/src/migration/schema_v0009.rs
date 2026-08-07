use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions ADD COLUMN title TEXT;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE sessions DROP COLUMN title;
"#];

/// Adds the nullable persisted display title for sessions.
pub fn migration() -> Migration {
    Migration::new("0009", UP_STATEMENTS, DOWN_STATEMENTS)
}

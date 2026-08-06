use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE agents ADD COLUMN content TEXT NOT NULL DEFAULT '';
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
ALTER TABLE agents DROP COLUMN content;
"#];

/// Adds the imported Markdown source retained by configurable agents.
pub fn migration() -> Migration {
    Migration::new("0008", UP_STATEMENTS, DOWN_STATEMENTS)
}

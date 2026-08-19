use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE plugin_state (
    plugin_id TEXT PRIMARY KEY NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE plugin_state;
"#];

/// Installs durable plugin eligibility without duplicating filesystem-derived package identity.
pub fn migration() -> Migration {
    Migration::new("0007", UP_STATEMENTS, DOWN_STATEMENTS)
}

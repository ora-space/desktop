use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE plugin_marketplace_source (
    url        TEXT PRIMARY KEY NOT NULL,
    branch     TEXT NOT NULL,
    use_proxy  INTEGER NOT NULL DEFAULT 0 CHECK (use_proxy IN (0, 1)),
    position   INTEGER NOT NULL CHECK (position >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX plugin_marketplace_source_position_unique
    ON plugin_marketplace_source(position);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP INDEX IF EXISTS plugin_marketplace_source_position_unique;
DROP TABLE IF EXISTS plugin_marketplace_source;
"#];

/// Builds durable plugin marketplace source configuration.
pub fn migration() -> Migration {
    Migration::new("0005", UP_STATEMENTS, DOWN_STATEMENTS)
}

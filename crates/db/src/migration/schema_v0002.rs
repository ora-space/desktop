use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE skills (
    id          TEXT PRIMARY KEY,
    namespace   TEXT NOT NULL DEFAULT 'local',
    name        TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    is_deleted  INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX skills_active_namespace_name_unique
    ON skills(namespace COLLATE NOCASE, name COLLATE NOCASE)
    WHERE is_deleted = 0;

CREATE TABLE agents (
    id          TEXT PRIMARY KEY,
    namespace   TEXT NOT NULL DEFAULT 'local',
    name        TEXT NOT NULL,
    description TEXT NOT NULL,
    content     TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    is_deleted  INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX agents_active_namespace_name_unique
    ON agents(namespace COLLATE NOCASE, name COLLATE NOCASE)
    WHERE is_deleted = 0;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS agents;
DROP TABLE IF EXISTS skills;
"#];

/// Builds the skill and configurable-agent catalog schema.
pub fn migration() -> Migration {
    Migration::new("0002", UP_STATEMENTS, DOWN_STATEMENTS)
}

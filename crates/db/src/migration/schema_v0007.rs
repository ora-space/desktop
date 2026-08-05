use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
CREATE TABLE project_spec_source_overrides (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    relative_path TEXT NOT NULL CHECK (length(trim(relative_path)) > 0),
    workflow_kind TEXT NOT NULL CHECK (workflow_kind IN ('open_spec', 'superpowers', 'custom')),
    custom_name TEXT,
    visibility TEXT NOT NULL CHECK (visibility IN ('enabled', 'disabled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
    FOREIGN KEY(project_id) REFERENCES projects(id),
    CHECK (
        (workflow_kind = 'custom' AND custom_name IS NOT NULL AND length(trim(custom_name)) > 0)
        OR (workflow_kind != 'custom' AND custom_name IS NULL)
    )
);
"#,
    r#"
CREATE UNIQUE INDEX idx_project_spec_source_overrides_active_path
ON project_spec_source_overrides(project_id, relative_path)
WHERE is_deleted = 0;
"#,
];

const DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX idx_project_spec_source_overrides_active_path;",
    "DROP TABLE project_spec_source_overrides;",
];

/// Builds the migration that persists project-wide specification source overrides.
pub fn migration() -> Migration {
    Migration::new("0007", UP_STATEMENTS, DOWN_STATEMENTS)
}

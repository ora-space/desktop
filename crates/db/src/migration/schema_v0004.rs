use super::Migration;

// Keeping schema removal as a tail migration ensures databases created by older releases converge
// on the same feature-free schema as fresh installations.
const UP_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS project_work_contexts;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
CREATE TABLE IF NOT EXISTS project_work_contexts (
    id TEXT PRIMARY KEY,
    surface TEXT NOT NULL,
    window_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    lease_expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_project_work_contexts_surface_window
    ON project_work_contexts (surface, window_id);

CREATE INDEX IF NOT EXISTS idx_project_work_contexts_project_lease
    ON project_work_contexts (project_id, lease_expires_at, surface, window_id);

CREATE INDEX IF NOT EXISTS idx_project_work_contexts_expiry
    ON project_work_contexts (lease_expires_at);
"#];

/// Builds the migration that removes persisted project work context state.
pub fn migration() -> Migration {
    Migration::new("0004", UP_STATEMENTS, DOWN_STATEMENTS)
}

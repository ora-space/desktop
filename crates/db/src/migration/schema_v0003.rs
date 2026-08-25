use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE workflows (
    id                    TEXT PRIMARY KEY,
    namespace             TEXT NOT NULL DEFAULT 'local',
    name                  TEXT NOT NULL,
    published_snapshot_id TEXT,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    is_deleted            INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE UNIQUE INDEX workflows_active_namespace_name_unique
    ON workflows(namespace COLLATE NOCASE, name COLLATE NOCASE)
    WHERE is_deleted = 0;

CREATE TABLE workflow_snapshots (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id),
    version     TEXT NOT NULL,
    graph       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER,
    is_deleted  INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE UNIQUE INDEX workflow_snapshots_active_version_unique
    ON workflow_snapshots(workflow_id, version)
    WHERE is_deleted = 0;

CREATE TABLE workflow_runs (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    workflow_id  TEXT NOT NULL REFERENCES workflows(id),
    snapshot_id  TEXT NOT NULL REFERENCES workflow_snapshots(id),
    name         TEXT NOT NULL,
    run_status   INTEGER NOT NULL,
    state        TEXT,
    input        TEXT,
    output       TEXT,
    error        TEXT,
    payload      TEXT,
    started_at   INTEGER,
    finished_at  INTEGER,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    is_deleted   INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE INDEX idx_workflow_runs_workspace
    ON workflow_runs(workspace_id, created_at, id);

CREATE INDEX idx_workflow_runs_workflow
    ON workflow_runs(workflow_id, created_at, id);

CREATE TABLE workflow_node_runs (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL REFERENCES workflow_runs(id),
    node_id     TEXT NOT NULL,
    node_type   TEXT NOT NULL,
    session_id  TEXT,
    status      INTEGER NOT NULL,
    input       TEXT,
    output      TEXT,
    error       TEXT,
    payload     TEXT,
    started_at  INTEGER,
    finished_at INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    is_deleted  INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE INDEX idx_workflow_node_runs_run
    ON workflow_node_runs(run_id, created_at, id);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS workflow_node_runs;
DROP TABLE IF EXISTS workflow_runs;
DROP TABLE IF EXISTS workflow_snapshots;
DROP TABLE IF EXISTS workflows;
"#];

/// Builds workflow definitions, snapshots, and execution state after workspace ownership exists.
pub fn migration() -> Migration {
    Migration::new("0003", UP_STATEMENTS, DOWN_STATEMENTS)
}

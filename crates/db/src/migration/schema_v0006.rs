use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
CREATE TABLE IF NOT EXISTS workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    published_snapshot_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    is_deleted INTEGER NOT NULL DEFAULT 0
        CHECK (is_deleted IN (0, 1))
);

CREATE TABLE IF NOT EXISTS workflow_snapshots (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    version TEXT NOT NULL,
    graph TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER,
    is_deleted INTEGER NOT NULL DEFAULT 0
        CHECK (is_deleted IN (0, 1)),
    FOREIGN KEY(workflow_id)
        REFERENCES workflows(id)
);

CREATE UNIQUE INDEX workflow_snapshots_active_version_unique
    ON workflow_snapshots(workflow_id, version)
    WHERE is_deleted = 0;
"#,
    r#"
CREATE TABLE IF NOT EXISTS workflow_runs (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id),
    snapshot_id TEXT NOT NULL REFERENCES workflow_snapshots(id),
    run_status  INTEGER NOT NULL,            -- 0=Pending 1=Running 2=Succeeded 3=Failed 4=Cancelled
    state       TEXT,                        -- JSON: {"current_nodes":[...]}
    input       TEXT,
    output      TEXT,
    error       TEXT,
    payload     TEXT,
    started_at  INTEGER,
    finished_at INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    is_deleted  INTEGER NOT NULL DEFAULT 0
        CHECK (is_deleted IN (0, 1))
);

CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow
    ON workflow_runs (workflow_id, created_at);

CREATE TABLE IF NOT EXISTS workflow_node_runs (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL REFERENCES workflow_runs(id),
    node_id     TEXT NOT NULL,
    node_type   TEXT NOT NULL,               -- start/agent/prompt/condition/tool/output
    session_id  TEXT,
    status      INTEGER NOT NULL,            -- 0=Pending 1=Running 2=Succeeded 3=Failed 4=Cancelled
    input       TEXT,
    output      TEXT,
    error       TEXT,
    payload     TEXT,
    started_at  INTEGER,
    finished_at INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    is_deleted  INTEGER NOT NULL DEFAULT 0
        CHECK (is_deleted IN (0, 1))
);

CREATE INDEX IF NOT EXISTS idx_workflow_node_runs_run
    ON workflow_node_runs (run_id, created_at);

ALTER TABLE tasks ADD COLUMN type INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN workflow_run_id TEXT REFERENCES workflow_runs(id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_workflow_run_id
    ON tasks (workflow_run_id) WHERE workflow_run_id IS NOT NULL;
"#,
];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP INDEX IF EXISTS idx_tasks_workflow_run_id;
ALTER TABLE tasks DROP COLUMN workflow_run_id;
ALTER TABLE tasks DROP COLUMN type;
DROP TABLE IF EXISTS workflow_node_runs;
DROP TABLE IF EXISTS workflow_runs;
DROP TABLE IF EXISTS workflow_snapshots;
DROP TABLE IF EXISTS workflows;
"#];

/// Builds the workflow definition and snapshot version migration.
pub fn migration() -> Migration {
    Migration::new("0006", UP_STATEMENTS, DOWN_STATEMENTS)
}

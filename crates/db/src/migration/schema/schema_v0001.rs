use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE user_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE projects (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    repository_kind TEXT NOT NULL DEFAULT 'git',
    repository_url  TEXT,
    default_branch  TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    is_deleted      INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE TABLE workspace_locations (
    id              TEXT PRIMARY KEY,
    location_kind   TEXT NOT NULL,
    plugin_id       TEXT,
    locator_version INTEGER NOT NULL DEFAULT 1 CHECK (locator_version > 0),
    locator_json    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK (
        (location_kind = 'remote_target' AND plugin_id IS NOT NULL)
        OR (location_kind <> 'remote_target' AND plugin_id IS NULL)
    )
);

CREATE TABLE workspaces (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id),
    workspace_kind  TEXT NOT NULL CHECK (workspace_kind IN ('main', 'isolated')),
    location_id     TEXT NOT NULL REFERENCES workspace_locations(id),
    lifecycle       TEXT NOT NULL DEFAULT 'provisioning',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    is_deleted      INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE UNIQUE INDEX workspaces_active_project_main_unique
    ON workspaces(project_id)
    WHERE workspace_kind = 'main' AND is_deleted = 0;

CREATE INDEX idx_workspaces_project
    ON workspaces(project_id, created_at, id);

CREATE TABLE workspace_provisioning (
    workspace_id           TEXT PRIMARY KEY REFERENCES workspaces(id),
    provisioner_kind       TEXT NOT NULL,
    plugin_id              TEXT,
    requested_revision     TEXT,
    requested_branch       TEXT,
    actual_revision        TEXT,
    actual_branch          TEXT,
    requested_locator_json TEXT,
    actual_locator_json    TEXT,
    state                  TEXT NOT NULL,
    last_error_code        TEXT,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    CHECK (
        (provisioner_kind = 'remote_target' AND plugin_id IS NOT NULL)
        OR (provisioner_kind <> 'remote_target' AND plugin_id IS NULL)
    )
);

CREATE TABLE worktrees (
    workspace_id    TEXT PRIMARY KEY REFERENCES workspaces(id),
    branch_name     TEXT,
    base_commit_id  TEXT CHECK (base_commit_id IS NULL OR base_commit_id <> ''),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    is_deleted      INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

-- Task is a user-facing label for an isolated worktree workspace. Runtime records never use it
-- to resolve project, worktree, session, or workflow-run ownership.
CREATE TABLE tasks (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL UNIQUE REFERENCES workspaces(id),
    title        TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    is_deleted   INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE TABLE sessions (
    id                      TEXT PRIMARY KEY,
    workspace_id            TEXT NOT NULL REFERENCES workspaces(id),
    title                   TEXT,
    agent_cli               TEXT NOT NULL,
    agent_session_id        TEXT NOT NULL,
    history_degraded_reason TEXT,
    status                  INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    is_deleted              INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
);

CREATE INDEX idx_sessions_workspace
    ON sessions(workspace_id, created_at, id);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS user_config;
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS tasks;
DROP TABLE IF EXISTS worktrees;
DROP TABLE IF EXISTS workspace_provisioning;
DROP TABLE IF EXISTS workspaces;
DROP TABLE IF EXISTS workspace_locations;
DROP TABLE IF EXISTS projects;
"#];

/// Builds user configuration plus the project, workspace, worktree, task, and session foundation.
pub fn migration() -> Migration {
    Migration::new("0001", UP_STATEMENTS, DOWN_STATEMENTS)
}

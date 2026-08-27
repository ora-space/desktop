use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE git_cleanup_jobs (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL CHECK (length(project_id) > 0),
    workspace_id    TEXT NOT NULL CHECK (length(workspace_id) > 0),
    repository_root TEXT NOT NULL CHECK (length(repository_root) > 0),
    checkout_root   TEXT,
    branch_name     TEXT NOT NULL CHECK (length(branch_name) > 0),
    state           TEXT NOT NULL CHECK (state IN ('pending', 'completed', 'manual_attention')),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at INTEGER NOT NULL,
    last_attempt_at INTEGER,
    last_error      TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX idx_git_cleanup_jobs_dispatch
    ON git_cleanup_jobs(state, next_attempt_at, id);

CREATE TABLE worktree_provisioning_leases (
    id               TEXT PRIMARY KEY,
    project_id       TEXT NOT NULL CHECK (length(project_id) > 0),
    workspace_id     TEXT NOT NULL CHECK (length(workspace_id) > 0),
    repository_root  TEXT NOT NULL CHECK (length(repository_root) > 0),
    checkout_root    TEXT NOT NULL CHECK (length(checkout_root) > 0),
    branch_name      TEXT NOT NULL CHECK (length(branch_name) > 0),
    lease_expires_at INTEGER NOT NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS worktree_provisioning_leases;
DROP TABLE IF EXISTS git_cleanup_jobs;
"#];

/// Builds durable Git cleanup jobs and worktree provisioning leases.
pub fn migration() -> Migration {
    Migration::new("0004", UP_STATEMENTS, DOWN_STATEMENTS)
}

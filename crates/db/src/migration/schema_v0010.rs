use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
ALTER TABLE worktrees ADD COLUMN checkout_root TEXT;
"#,
    r#"
CREATE TABLE IF NOT EXISTS git_cleanup_jobs (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL CHECK (length(project_id) > 0),
    task_id         TEXT NOT NULL CHECK (length(task_id) > 0),
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
"#,
    r#"
CREATE INDEX IF NOT EXISTS idx_git_cleanup_jobs_dispatch
ON git_cleanup_jobs(state, next_attempt_at, id);
"#,
    r#"
CREATE TABLE IF NOT EXISTS worktree_provisioning_leases (
    id               TEXT PRIMARY KEY,
    project_id       TEXT NOT NULL CHECK (length(project_id) > 0),
    task_id          TEXT NOT NULL CHECK (length(task_id) > 0),
    repository_root  TEXT NOT NULL CHECK (length(repository_root) > 0),
    checkout_root    TEXT NOT NULL CHECK (length(checkout_root) > 0),
    branch_name      TEXT NOT NULL CHECK (length(branch_name) > 0),
    lease_expires_at INTEGER NOT NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
"#,
];

const DOWN_STATEMENTS: &[&str] = &[
    r#"
DROP TABLE worktree_provisioning_leases;
"#,
    r#"
DROP INDEX idx_git_cleanup_jobs_dispatch;
"#,
    r#"
DROP TABLE git_cleanup_jobs;
"#,
    r#"
ALTER TABLE worktrees DROP COLUMN checkout_root;
"#,
];

/// Adds durable Git cleanup jobs, worktree provisioning leases, and the
/// persisted checkout path that serves as cleanup ownership evidence.
///
/// Rolling back drops all pending cleanup and provisioning bookkeeping, which
/// intentionally re-accepts the pre-migration behavior of leaking physical Git
/// resources on aggregate deletion.
pub fn migration() -> Migration {
    Migration::new("0010", UP_STATEMENTS, DOWN_STATEMENTS)
}

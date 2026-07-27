use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
ALTER TABLE worktrees ADD COLUMN base_commit_id TEXT;

CREATE TABLE task_diff_comments (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    parent_comment_id TEXT,
    diff_id TEXT,
    path TEXT,
    side INTEGER,
    start_line INTEGER,
    end_line INTEGER,
    hunk_header TEXT,
    line_content TEXT,
    body TEXT NOT NULL,
    status INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
    UNIQUE (id, task_id),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (parent_comment_id, task_id)
        REFERENCES task_diff_comments(id, task_id),
    CHECK (
        (
            parent_comment_id IS NULL
            AND diff_id IS NOT NULL
            AND path IS NOT NULL
            AND side IN (0, 1)
            AND start_line > 0
            AND end_line >= start_line
            AND hunk_header IS NOT NULL
            AND line_content IS NOT NULL
            AND status IN (0, 1)
        )
        OR
        (
            parent_comment_id IS NOT NULL
            AND diff_id IS NULL
            AND path IS NULL
            AND side IS NULL
            AND start_line IS NULL
            AND end_line IS NULL
            AND hunk_header IS NULL
            AND line_content IS NULL
            AND status IS NULL
        )
    )
);

CREATE INDEX idx_task_diff_comments_task
    ON task_diff_comments (task_id, created_at, id);

CREATE INDEX idx_task_diff_comments_parent
    ON task_diff_comments (parent_comment_id, created_at, id);

CREATE TRIGGER task_diff_comments_parent_must_be_thread
BEFORE INSERT ON task_diff_comments
WHEN NEW.parent_comment_id IS NOT NULL
BEGIN
    SELECT CASE WHEN EXISTS (
        SELECT 1
        FROM task_diff_comments AS parent
        WHERE parent.id = NEW.parent_comment_id
          AND parent.parent_comment_id IS NOT NULL
    ) THEN RAISE(ABORT, 'task diff reply parent must be a root thread') END;
END;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TRIGGER IF EXISTS task_diff_comments_parent_must_be_thread;
DROP INDEX IF EXISTS idx_task_diff_comments_parent;
DROP INDEX IF EXISTS idx_task_diff_comments_task;
DROP TABLE IF EXISTS task_diff_comments;
ALTER TABLE worktrees DROP COLUMN base_commit_id;
"#];

/// Builds the task diff baseline and comment persistence migration.
pub fn migration() -> Migration {
    Migration::new("0004", UP_STATEMENTS, DOWN_STATEMENTS)
}

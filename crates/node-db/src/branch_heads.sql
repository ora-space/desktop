-- The last value of each own Workspace branch a checkout is known to have on the Git Server,
-- written after an own push or fetch succeeded. Agent-writable refs never feed it.
CREATE TABLE workspace_branch_heads (
    checkout TEXT NOT NULL REFERENCES clone_executions(execution),
    ref TEXT NOT NULL CHECK(ref LIKE 'refs/heads/workspaces/%'),
    commit_id TEXT NOT NULL CHECK(length(commit_id) IN (40, 64)),
    PRIMARY KEY(checkout, ref)
);

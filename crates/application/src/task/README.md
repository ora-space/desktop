# Task Application Module

This module coordinates task CRUD with optional backend-owned Git worktree provisioning.

## Creation modes

- Root-mode tasks use the project checkout directly and persist no worktree id.
- Worktree-mode tasks require a valid Git repository and a selected local base ref, reserve a non-colliding task id/branch prefix, resolve that local ref to an immutable commit id, create a linked worktree from that commit, persist only that commit id in the `Worktree` record, and then persist the `Task` that owns it.
- Worktree paths are composed only during creation from the configured worktree root and full task id. Existing paths are resolved from persisted branch identity and Git metadata elsewhere.
- If persistence fails after Git resources are created, the handler attempts compensating soft deletion and forced worktree cleanup while preserving the original stable application error.

## Boundaries and invariants

`TaskRepository`, `WorktreeRepository`, identifier generators, `Clock`, and `TaskWorktreeProvisioner` keep database and Git details outside the use-case logic. `GitTaskWorktreeProvisioner` adapts the typed `gitlancer` runtime to that port.

Task updates preserve project ownership and the existing worktree association. Aggregate deletion is handled by backend/database cascade logic, which registers durable Git cleanup jobs in the deletion transaction; this module supplies the cleanup vocabulary the backend worker executes — identity validation, the `TaskGitResourceCleaner` port with its Git implementation, and the pure reduction from stage outcomes to job transitions.

Branch creation uses a short task-id prefix, so creation checks both existing task worktree directories and repository branches before accepting an id. Worktree mode fails explicitly when the project root is not a Git repository.

The frontend lists local project refs before creation. Ora-managed `ora/<prefix>` branches retain their Git identity in requests but use the owning task title as their display label, so an existing worktree can seed another one without any implicit remote refresh.

See the [ora-application overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts-boundary.md).

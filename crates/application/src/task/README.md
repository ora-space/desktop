# Task Application Module

This module coordinates task CRUD with optional backend-owned Git worktree provisioning.

## Creation modes

- Root-mode tasks use the project checkout directly and persist no worktree id.
- Worktree-mode tasks require a valid Git repository, reserve a non-colliding task id/branch prefix, create a linked worktree, persist its `Worktree` record, and then persist the `Task` that owns it.
- Worktree paths are composed only during creation from the configured worktree root and full task id. Existing paths are resolved from persisted branch identity and Git metadata elsewhere.
- If persistence fails after Git resources are created, the handler attempts compensating soft deletion and forced worktree cleanup while preserving the original stable application error.

## Boundaries and invariants

`TaskRepository`, `WorktreeRepository`, identifier generators, `Clock`, and `TaskWorktreeProvisioner` keep database and Git details outside the use-case logic. `GitTaskWorktreeProvisioner` adapts the typed `gitlancer` runtime to that port.

Task updates preserve project ownership and the existing worktree association. Aggregate deletion is handled by backend/database cascade logic and deliberately does not remove Git branches or worktrees.

Branch creation uses a short task-id prefix, so creation checks both existing task worktree directories and repository branches before accepting an id. Worktree mode fails explicitly when the project root is not a Git repository.

See the [ora-application overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts.md).

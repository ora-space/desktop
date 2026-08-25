# Task Application Module

This module coordinates the user-facing Task projection for isolated Workspace creation and
backend-owned Git worktree provisioning.

## Creation modes

- Task creation always provisions an isolated Workspace backed by a linked Git worktree. It
  requires a valid Git repository and a selected local base ref, creates independent Task and
  Workspace identities, reserves a non-colliding Workspace branch prefix, resolves that local ref
  to an immutable commit id, creates the linked worktree from that commit, persists only that
  commit id in the `Worktree` record, and then persists the user-facing `Task` projection.
- Worktree paths are composed only during creation from the configured worktree root and full Workspace id. Existing paths are resolved from persisted Workspace location evidence and Git metadata elsewhere.
- If persistence fails after Git resources are created, the handler attempts compensating soft deletion and forced worktree cleanup while preserving the original stable application error.

## Boundaries and invariants

`TaskRepository`, `WorktreeRepository`, identifier generators, `Clock`, and `TaskWorktreeProvisioner` keep database and Git details outside the use-case logic. `GitTaskWorktreeProvisioner` adapts the typed `gitlancer` runtime to that port.

Task updates preserve project ownership and the existing worktree association. Aggregate deletion is handled by backend/database cascade logic, which registers durable Git cleanup jobs in the deletion transaction; this module supplies the cleanup vocabulary the backend worker executes — identity validation, the `TaskGitResourceCleaner` port with its Git implementation, and the pure reduction from stage outcomes to job transitions.

Branch creation uses a short Workspace-id prefix, so creation checks both existing worktree directories and repository branches before accepting an identity. Worktree mode fails explicitly when the main Workspace location is not a Git repository.

The frontend lists local project refs before creation. Ora-managed `ora/<prefix>` branches retain their Git identity in requests but use the owning task title as their display label, so an existing worktree can seed another one without any implicit remote refresh.

See the [ora-application overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts-boundary.md).

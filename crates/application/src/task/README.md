# Task Application Module

This module coordinates task CRUD with optional backend-owned Git worktree provisioning.

## Creation modes

- Root-mode tasks use the project checkout directly and persist no worktree id.
- Worktree-mode tasks require a valid Git repository, reserve a non-colliding task id/branch prefix, create a linked worktree, persist its `Worktree` record, and then persist the `Task` that owns it.
- Worktree paths are composed only during creation from the configured worktree root and full task id. Existing paths are resolved from persisted branch identity and Git metadata elsewhere.
- If persistence fails after Git resources are created, the handler attempts compensating soft deletion and forced worktree cleanup while preserving the original stable application error.

## Boundaries and invariants

`TaskRepository`, `WorktreeRepository`, identifier generators, `Clock`, `TaskWorktreeProvisioner`, and `TaskGitResourceCleaner` keep database and Git details outside orchestration policy. `GitTaskWorktreeProvisioner` owns creation and creation-failure compensation. `GitTaskGitResourceCleaner` independently reports aggregate worktree and branch cleanup outcomes so the backend can apply best-effort logging after a database commit.

Task updates preserve project ownership and the existing worktree association. Aggregate deletion timing and database cascading remain backend/database responsibilities. Cleanup resolves worktrees by validated branch metadata first and accepts only an exact deterministic checkout root as its detached fallback; it never selects a parent checkout or the main worktree. The shared task-id-derived branch naming function lets creation and destructive-cleanup ownership validation enforce the same `ora/<first-eight-task-id-characters>` invariant.

Branch creation uses a short task-id prefix, so creation checks both existing task worktree directories and repository branches before accepting an id. Worktree mode fails explicitly when the project root is not a Git repository.

See the [ora-application overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts.md).

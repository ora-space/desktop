# Worktree Application Module

This module defines the application-facing persistence seam for backend-owned task worktrees.

`WorktreeRepository` provides visible-record CRUD and soft deletion for domain `Worktree` values. A worktree is identified by its owning `WorkspaceId`; it has no second identity independent from the workspace.

The module intentionally has no standalone handlers or transport contracts. Worktree records are internal metadata coordinated by the task application module and backend session path; they are not an independent public CRUD resource.

Repository implementations must preserve stored branch identity and soft-delete semantics. They must not reconstruct checkout paths, invoke Git, or decide when a worktree should be created or removed. Git lifecycle belongs to `TaskWorktreeProvisioner`, while concrete persistence belongs to `ora-db`.

See the [ora-application overview](../../README.md) and [Task Application Module](../task/README.md).

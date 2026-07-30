# Task Worktrees

A task's filesystem context is backend-owned state. Callers choose a workspace mode at creation; they never see or supply a worktree identifier.

## Workspace modes

`CreateTaskRequest.workspace_mode` selects between two creation paths and defaults to `Worktree` when omitted:

- **`worktree`** — the backend provisions one linked Git worktree owned by the task, persists a `Worktree` record for that checkout, and persists the task with the resulting internal `worktree_id`.
- **`project_root`** — the task uses the owning project's checkout directly. No Git worktree is created and no worktree record is persisted.

The public `Task` payload exposes `workspaceMode`, not `worktreeId`. `CreateTaskRequest` and `UpdateTaskRequest` accept no worktree identifier, and updates preserve both project ownership and the existing worktree association.

## Provisioning a worktree-mode task

`CreateTaskHandler` orchestrates the whole flow behind the `TaskWorktreeProvisioner` port, so no Git type reaches a request or response contract:

1. Validate that the project root is a Git repository. Worktree mode fails explicitly when it is not.
2. Reserve a task identifier whose short branch prefix does not collide (below).
3. Derive the branch name and the worktree directory from that identifier.
4. Create the linked worktree through the provisioner.
5. Persist the `Worktree` record, then persist the `Task` that owns it.

Branch names use the first **8** characters of the task id as `ora/<prefix>`, while the worktree directory uses the **full** task id under the configured worktree root: `<worktree_root>/<task_id>`.

Because the branch name is shortened, collision checking has to cover both places the prefix can already be taken. Before accepting a generated id the handler rejects it if either an existing task worktree directory starts with that prefix, or a local `ora/<prefix>` branch already exists. An orphaned branch whose checkout directory was removed therefore still reserves its prefix. After a bounded number of attempts the handler fails rather than looping.

`GitTaskWorktreeProvisioner` adapts the typed `gitlancer` runtime to the port. A unit test can substitute a fake provisioner and exercise the complete create flow with no Git repository or filesystem side effects.

## Failure handling

Git and database state must not drift apart, and a partially created workspace is never exposed.

- If linked-worktree creation fails, the handler returns a stable application error and persists no task or worktree row.
- If persistence fails *after* the Git worktree was created, the handler attempts compensating cleanup — soft-deleting whatever record was written and removing the linked worktree with `TaskWorktreeDeletionMode::Force`, so a dirty checkout cannot block the rollback — and then returns the original application error rather than a cleanup error.

The Web runtime maps these into structured HTTP server errors that identify task creation as failed without exposing raw Git command output or filesystem formatting.

## Path resolution after creation

The configured worktree root is a **creation target only**. It affects task creations that begin after it is updated; in-flight operations keep their original snapshot, and existing worktrees are never moved.

Existing checkout paths are never recomposed from the configured root. When an agent session starts or loads, the path is resolved live: task → persisted `Worktree` id → stored branch name → `git worktree list --porcelain`, which is the authoritative source. `Backend::resolve_task_cwd` reuses that same resolution so any caller sees the directory the session actually runs in.

## Deletion

Task and project deletion remove Ora-owned database records only. `SqliteCascadeRepository` soft-deletes the aggregate — sessions and the owned worktree record — in one transaction, and rejects the operation with `resource_in_use` when a descendant session is still `Running`.

These paths deliberately **do not** call Git. The linked worktree directory and its `ora/<prefix>` branch survive task deletion, and provider-owned ACP history is never deleted. Forced worktree removal happens only as compensating cleanup for a failed creation.

See [Application and Contracts Boundary](application-contracts.md), [Gitlancer Architecture](gitlancer-architecture.md), and [ACP Agent Runtime](agent-runtime.md).

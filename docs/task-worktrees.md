# Task Worktrees

A task is the user-facing label for an isolated workspace. Its filesystem context is backend-owned state, and callers never see or supply a separate worktree identifier.

## Workspace ownership

`CreateTaskRequest` names the project, title, and base branch. The backend creates a dedicated Workspace and linked Git worktree for the task.

The public `Task` payload exposes `workspaceId`. The `worktrees.workspace_id` shared primary key makes the Git metadata an optional one-to-one extension of that Workspace instead of introducing another identity. Updates preserve workspace ownership, and `CreateTaskRequest.baseBranch` is required.

## Provisioning a worktree-mode task

`CreateTaskHandler` orchestrates the whole flow behind the `TaskWorktreeProvisioner` port, so no Git type reaches a request or response contract:

1. Validate that the project root is a Git repository. Worktree mode fails explicitly when it is not.
2. Resolve the selected local base ref to an immutable commit id.
3. Reserve independent Task and Workspace identifiers whose short Workspace branch prefix does not collide (below).
4. Derive the branch name and the worktree directory from the Workspace identifier.
5. Create the linked worktree and its branch at the resolved commit.
6. Persist the Workspace, its provisioning result, the `Worktree` extension, and the `Task` label in one transaction.

Branch names use the first **8** characters of the Workspace id as `ora/<prefix>`, while the worktree directory uses the **full** Workspace id under the configured worktree root: `<worktree_root>/<workspace_id>`.

Because the branch name is shortened, collision checking has to cover both places the prefix can already be taken. Before accepting a generated id the handler rejects it if either an existing task worktree directory starts with that prefix, or a local `ora/<prefix>` branch already exists. An orphaned branch whose checkout directory was removed therefore still reserves its prefix. After a bounded number of attempts the handler fails rather than looping.

`GitTaskWorktreeProvisioner` adapts the typed `gitlancer` runtime to the port. A unit test can substitute a fake provisioner and exercise the complete create flow with no Git repository or filesystem side effects.

## Base selection and display labels

Gitlancer lists only local branches from `refs/heads`. Opening the selector and creating a worktree never fetches, prunes, or otherwise changes remote-tracking refs, so local worktree creation remains available offline and always uses the branch state currently available in the repository.

The application layer joins the available refs with project-owned task and worktree records. For an `ora/<prefix>` branch it returns the owning task title as `displayName`, while retaining the exact `refName` that Git must resolve. Creating a worktree invalidates that project's branch-list cache so a newly created Ora branch is immediately available as the next task's base.

## Failure handling

Git and database state must not drift apart, and a partially created workspace is never exposed.

- If linked-worktree creation fails, the handler returns a stable application error and persists no task or worktree row.
- If the selected local base ref no longer exists, the handler returns `base_branch_not_found` before creating a task branch or worktree.
- Before `git worktree add` runs, the handler persists a **provisioning lease** carrying the exact repository root, checkout path, and branch. The lease is renewed while slow Git work runs and is deleted inside the same transaction that commits the task and worktree rows, so at every instant the provisioned Git resources have exactly one durable owner: the lease, or the committed rows.
- The commit itself is a single-transaction unit of work (`SqliteTaskWorkspaceRepository`) that re-validates the owning project is still visible. Losing the race against a project deletion returns `project_not_found` and hands the lease to durable cleanup.
- If anything fails after the Git worktree was created — persistence errors, the project disappearing, or a process crash — the provisioned worktree and branch are reclaimed through the durable Git cleanup path: either the handler releases the lease into a cleanup job immediately, or the lease expires without renewal and the cleanup worker converts it.

The Desktop runtime maps these into typed `ContractError` values that identify task creation as failed without exposing raw Git command output or filesystem formatting.

## Path resolution after creation

The configured worktree root is a **creation target only**. It affects task creations that begin after it is updated; in-flight operations keep their original snapshot, and existing worktrees are never moved.

Existing checkout paths are never recomposed from the configured root. When an agent session starts or loads, the path is resolved live: task → Workspace id → stored Worktree branch name → `git worktree list --porcelain`, which is the authoritative source. `Backend::resolve_task_cwd` reuses that same resolution so any caller sees the directory the session actually runs in.

## Deletion

Task, project, and workflow-run deletion share one semantic: **the database cascade is the success, and physical Git cleanup is durable and asynchronous**. `SqliteCascadeRepository` soft-deletes the aggregate — sessions and the owned worktree record — in one transaction, and rejects the operation with `resource_in_use` when a descendant session is still `Running`.

Inside that same transaction the cascade reads each worktree-backed task's persisted identity (repository root, branch name, recorded checkout path) and inserts one `git_cleanup_jobs` row per task. Because the jobs commit atomically with the soft deletes, a crash, power loss, or SIGKILL after the commit can never lose the cleanup targets: the backend's Git cleanup worker replays every pending job on the next start.

The worker force-removes the linked worktree (resolved by branch first, then by the recorded checkout path for detached worktrees) and force-deletes the local `ora/<prefix>` branch. Both stages are idempotent — an already-absent resource is a positively confirmed success, never an error. Removal is only confirmed on disk: when Git deregisters a worktree but leaves the directory behind (a common Windows half-failure), an empty leftover shell is finished with a plain filesystem removal, while a leftover that still has content stays a retryable failure. A **non-empty** checkout that exists on disk but can no longer be proven Ora-owned is left untouched and the job parks as `manual_attention`; empty directories at a recorded checkout path hold no user data and are reclaimed as Ora residue. Bounded retries with backoff handle transient Git failures. Remote branches are never touched, and provider-owned ACP history is never deleted.

See [Application and Contracts Boundary](application-contracts-boundary.md), [Gitlancer Architecture](gitlancer-architecture.md), and [ACP Agent Runtime](agent-runtime.md).

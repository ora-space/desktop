# Tasks and Worktrees

English | [中文](tasks-and-worktrees.zh.md)

Every project has one **Main** Workspace: its own checkout. A **task** is the user-facing label for
an **Isolated** Workspace backed by a linked Git worktree. Sessions and workflow runs execute in a
Workspace; the worktree itself is backend-owned state, and no contract carries a worktree id or
offers worktree operations.

## Workspace ownership

The public `Task` payload exposes `workspaceId`. `worktrees.workspace_id` is both the primary key
and the Workspace foreign key, so the Git metadata is an optional one-to-one extension of the
Workspace rather than a second identity. `UpdateTaskRequest` cannot change project or Workspace
ownership, and `CreateTaskRequest.baseBranch` is required.

## Creating a task

`CreateTaskHandler` runs the whole flow behind the `TaskWorktreeProvisioner` port, so no Git type
reaches a contract and a unit test can substitute a fake provisioner:

1. Check that the project root is a Git repository; worktree mode fails explicitly when it is not.
   Desktop hides the new-worktree entries for such a project up front, using the `git` capability
   that `listWorkspaces` reports, see [Workspace review](workspace-review.md#folders-without-git).
2. Resolve the selected local base ref to an immutable commit id. A ref that no longer exists
   returns `base_branch_not_found` before anything is created.
3. Reserve Task and Workspace ids whose short branch prefix does not collide (below).
4. Persist a **provisioning lease** naming the exact repository root, checkout path and branch.
5. Create the branch and linked worktree at the resolved commit, renewing the lease while Git runs.
6. Commit the Workspace, its provisioning result, the `Worktree` row and the `Task` in one
   transaction (`SqliteTaskWorkspaceRepository`) that also re-validates that the project is still
   visible and deletes the lease.

The branch is `ora/<first 8 characters of the Workspace id>`; the checkout directory is
`<worktree root>/<full Workspace id>`. Because the branch name is shortened, a generated id is
rejected if any existing task worktree directory starts with that prefix or a local `ora/<prefix>`
branch already exists, so an orphaned branch whose checkout was removed still reserves its prefix.
After three attempts the handler fails instead of looping.

At every instant the provisioned Git resources have exactly one durable owner: the lease or the
committed rows. If anything fails after Git created them (a persistence error, the project being
deleted concurrently, which returns `project_not_found`, or a crash), the handler hands the lease to
durable cleanup, or the lease expires unrenewed and the cleanup worker converts it. A partially
created Workspace is never exposed, and Desktop reports a typed error without raw Git output or
paths.

## Base branches

Branch listing reads only local `refs/heads`. Opening the selector and creating a worktree never
fetch, prune or touch remote-tracking refs, so creation works offline and uses the branch state
already in the repository. `ListProjectBranchesHandler` joins the refs with the project's task and
worktree records: an `ora/<prefix>` branch keeps its exact `refName` but displays the owning task's
title. Creating a worktree invalidates the project's branch list so the new branch can immediately
seed the next task.

## Resolving a checkout

The configured worktree root is a creation target only. Changing it affects creations that start
afterwards; in-flight operations keep their snapshot and existing worktrees are never moved.
Existing paths are never recomposed from the root: a task's checkout resolves live through Task →
Workspace → stored branch name → `git worktree list --porcelain`. Agent sessions, workspace
review, `resolve_task_cwd` and `resolve_workspace_cwd` all use that resolution. A Main Workspace
resolves its stored location against the Desktop path base, see
[Desktop host](../desktop/desktop-host.md#persistent-paths).

## Reviewing large change sets

The Changes and Files controls stay available when a Git comparison is loading or fails.
If the complete patch exceeds the preview limit, Changes lists the files in the selected Git
scope and loads the selected file's diff with nearby context. A file that still exceeds the
limit remains listed, and other files can still be reviewed.

## Deletion and durable Git cleanup

Task, project and workflow-run deletion share one rule: **the database cascade is the success;
physical Git cleanup is durable and asynchronous**. `SqliteCascadeRepository` soft-deletes the
aggregate (sessions and the owned worktree record) in one transaction and rejects it with
`resource_in_use` while a descendant session is `Running`. In the same transaction it reads each
worktree-backed task's repository root, branch and recorded checkout path and inserts one
`git_cleanup_jobs` row per task, so a crash, power loss or SIGKILL after commit cannot lose the
cleanup targets.

The Backend's cleanup worker (`crates/backend/src/git_cleanup`) consumes pending jobs and expired
provisioning leases:

- It force-removes the linked worktree (resolved by branch, or by the recorded path for a detached
  worktree) and force-deletes the local `ora/<prefix>` branch. Both stages are idempotent: an
  already absent resource is a confirmed success.
- Removal is confirmed on disk. When Git deregisters a worktree but leaves the directory (a common
  Windows half-failure), an empty leftover is removed directly, while one with content stays a
  retryable failure. A non-empty checkout that can no longer be proven Ora-owned is left untouched
  and the job parks as `manual_attention`.
- Jobs have no persisted "running" state, so a crash at any point leaves them replayable; the first
  pass after start is the reconciliation, and a 60-second scan recovers lost wake-ups.
- A batch takes at most 16 jobs grouped by repository; up to four repositories run concurrently and
  jobs within one repository run serially. After five failed attempts with backoff a job parks as
  `manual_attention`.
- Short-lived checkout readers (diff, commit, push) hold a shared worktree-use lease that cleanup
  takes exclusively, and all worktree and branch mutations of one repository are serialized by a
  repository gate. The lock order is lease first, then gate.

Remote branches are never touched and provider-owned ACP history is never deleted. Cleanup
outcomes are logged under `operation = "git_cleanup"`, independent of the request that deleted the
task.

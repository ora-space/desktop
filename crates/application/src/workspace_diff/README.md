# Workspace Diff Application Module

This module owns the transport- and storage-independent use cases behind Git change review for a workspace checkout — an isolated task worktree or a project's main checkout alike. It keeps Git execution, SQLite rows, filesystem paths, and frontend rendering outside the application layer by defining small ports and composing generic handlers over them.

## Responsibilities

- Expose a Git-backed `WorkspaceDiffReader` that returns a workspace-scoped unified patch for backend composition.
- Commit and push changes for any workspace checkout, verifying the caller's persisted branch identity when one is recorded, and trusting the current checkout otherwise.

## Ports and adapters

`WorkspaceDiffReader` and `WorkspaceGitWriter` are the public seams used by handlers. They use static dispatch so tests can provide in-memory fakes without a Git process. The concrete `GitWorkspaceDiffReader` and `GitWorkspaceGitWriter` adapters are thin translators around `gitlancer`.

The module receives a backend-resolved worktree path keyed by `WorkspaceId` and never accepts a frontend filesystem path or a `TaskId`. Backend composition resolves the path (a project checkout or an isolated task worktree — both are `Workspace` rows) and looks up the optional `Worktree` row for that workspace before invoking a handler:

- A `Worktree` row exists (an isolated task worktree) → the handler verifies the checkout's path and branch against that recorded state before mutating, and a recorded baseline commit is required for the `Branch`/`Committed` diff scopes.
- No `Worktree` row exists (a project's main checkout, which Ora does not manage the way it manages a task worktree) → the handler skips verification and mutates whatever is currently checked out; no baseline exists, so only the `Unstaged`/`Staged` diff scopes are usable.

## Error and logging boundary

Validation failures are semantic `ApplicationError` variants, such as `WorkspaceDiffCommitMessageBlank`. Git and persistence failures retain boxed `Error::source()` chains through the application port and are projected to `internal_error` by `ora-backend`. Handlers do not emit request-completion logs; Web, Tauri, and stream adapters emit one correlated completion event through `RequestLifecycle`, where `ora-logging` bounds and redacts the diagnostic chain.

The diff reader enforces a bounded patch response. Oversized patches become the public `workspace_diff_too_large` payload, while the discarded byte count stays out of the public contract.

## Invariants

- A commit or push against a workspace with a recorded `Worktree` row verifies its path and branch against Git metadata immediately before mutation; a workspace with no such row has nothing to verify against.
- The `Branch`/`Committed` diff scopes require a recorded baseline commit; requesting them without one is rejected before Git ever runs.
- The application module does not choose adapter status, Tauri behavior, public error codes, or log levels.

See [Application and Contracts Boundary](../../../../docs/application-contracts-boundary.md) and [Task Worktrees](../../../../docs/task-worktrees.md).

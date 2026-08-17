# Task Diff Application Module

This module owns the transport- and storage-independent use cases behind task change review. It keeps Git execution, SQLite rows, filesystem paths, and frontend rendering outside the application layer by defining small ports and composing generic handlers over them.

## Responsibilities

- Read a task-scoped unified diff and calculate its stable `diff_id` from the base revision, current `HEAD`, and complete patch.
- Stage, commit, and push changes only after the caller's task, worktree, and persisted branch identity have been verified.
- Create root line discussions, replies, and resolution changes while making invalid comment states unrepresentable with `TaskDiffCommentKind`.
- Reject stale or structurally invalid anchors before persisting a comment.
- Normalize anchor paths through `ora-utils` portable relative-path validation before matching or
  persistence, so Git paths retain one platform-independent slash representation.
- Map domain comments into the shared `ora-contracts` DTOs.

## Ports and adapters

`TaskDiffReader`, `TaskGitWriter`, and `TaskDiffCommentRepository` are the public seams used by handlers. They use static dispatch so tests can provide in-memory fakes without a Git process or SQLite database. The concrete `GitTaskDiffReader` and `GitTaskGitWriter` adapters are thin translators around `gitlancer`; SQLite owns the comment repository implementation in `ora-db`.

The module receives a backend-resolved worktree path and never accepts a frontend filesystem path. Backend composition decides whether the path is the project checkout or an isolated task worktree and supplies the appropriate baseline before invoking a handler.

## Error and logging boundary

Validation failures are semantic `ApplicationError` variants, such as `TaskDiffStale`, `TaskDiffCommentInvalid`, and `TaskDiffCommitMessageBlank`. Git and persistence failures retain boxed `Error::source()` chains through the application port and are projected to `internal_error` by `ora-backend`. Handlers do not emit request-completion logs; Web, Tauri, and stream adapters emit one correlated completion event through `RequestLifecycle`, where `ora-logging` bounds and redacts the diagnostic chain.

The diff reader enforces a bounded patch response. Oversized patches become the public `task_diff_too_large` payload, while the discarded byte count stays out of the public contract.

## Invariants

- A root discussion owns its `TaskDiffAnchor` and thread status; a reply owns only its parent comment id.
- A comment can be created only when its `diff_id`, path, hunk, line range, side, and first-line content still match the current patch.
- Comment paths cannot be rooted, carry platform prefixes, or traverse to a parent component.
- Commit and push operations require an active task-owned worktree and verify its path and branch against Git metadata immediately before mutation.
- The application module does not choose HTTP status codes, Tauri behavior, public error codes, or log levels.

See [Application and Contracts Boundary](../../../../docs/application-contracts.md) and [Task Worktree / Gitlancer Diff Flow](../../../../docs/task-worktree-gitlancer-frontend-flow.md).

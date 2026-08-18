# SQLite Repository Module

This module implements `ora-application` persistence ports on SQLite and exposes the cloneable `RepositoryPool` used by backend composition.

## Responsibilities

- Concrete repositories map projects, tasks, sessions, skills, configurable agents, worktrees, and task diff comments between SQL rows and domain values.
- Normal reads exclude soft-deleted rows. Soft deletion records timestamps rather than removing individual domain records physically.
- `RepositoryPool` serializes access to its connection and gives repository operations a consistent error boundary.
- `SqliteTaskDiffCommentRepository` stores root threads and replies in one table while preserving the domain enum's mutually exclusive anchor shape. It never returns soft-deleted comments and orders visible messages by creation time and id.

## Aggregate deletion

`SqliteCascadeRepository` soft-deletes task or project aggregates in one immediate transaction. It rechecks existence and running descendants under the write lock so a session cannot become Running between validation and updates.

Task deletion cascades through its sessions and owned worktree record. Project deletion cascades through tasks, sessions, and worktree records. A running descendant returns `ResourceInUse`; no partial cascade is committed.

These transactions mutate Ora-owned database state only. They never invoke Git, remove checkout directories or branches, or delete provider-owned ACP history.

SQL details remain internal to this module; lifecycle policy and public error mapping belong to `ora-application` and `ora-backend`.

Repository failures preserve their concrete SQLite errors behind application-owned source-chain wrappers. Skill package promotion and compensation are outside this module, so SQLite never coordinates source copies or filesystem renames. This module does not stringify or log failures that the outer request lifecycle will complete.

See the [ora-db overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts-boundary.md).

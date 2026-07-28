# SQLite Repository Module

This module implements `ora-application` persistence ports on SQLite and exposes the cloneable `RepositoryPool` used by backend composition.

## Responsibilities

- Concrete repositories map projects, tasks, sessions, skills, configurable agents, worktrees, and project work contexts between SQL rows and domain values.
- Normal reads exclude soft-deleted rows. Soft deletion records timestamps rather than removing individual domain records physically.
- `RepositoryPool` serializes access to its connection and gives repository operations a consistent error boundary.
- Project work context queries preserve surface/window identity, lease expiry, and active-project lookup semantics.

## Aggregate deletion

`SqliteCascadeRepository` soft-deletes task or project aggregates in one immediate transaction. It rechecks existence and running descendants under the write lock so a session cannot become Running between validation and updates.

Task deletion cascades through its sessions and owned worktree record. Project deletion cascades through project contexts, tasks, sessions, and worktree records. A running descendant returns `ResourceInUse`; no partial cascade is committed.

These transactions mutate Ora-owned database state only. They never invoke Git, remove checkout directories or branches, or delete provider-owned ACP history.

SQL details remain internal to this module; lifecycle policy and public error mapping belong to `ora-application` and `ora-backend`.

See the [ora-db overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts.md).

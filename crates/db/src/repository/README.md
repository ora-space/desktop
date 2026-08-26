# SQLite Repository Module

This module implements `ora-application` persistence ports on SQLite and exposes the cloneable `RepositoryPool` used by backend composition.

## Responsibilities

- Concrete repositories map projects, workspaces, worktree-task labels, sessions, workflow runs,
  skills, configurable agents, plugin marketplace sources, worktrees, and typed user preferences
  between SQL rows and application values.
- `SqliteEffectRepository` stores normalized Desired selections, source state, surface descriptors,
  ownership ledgers, status, and durable operations. Desired replacement uses generation CAS;
  operation finalization changes its ledger and journal phase in one immediate transaction.
- Normal reads exclude soft-deleted rows. Soft deletion records timestamps rather than removing individual domain records physically.
- `RepositoryPool` serializes access to its connection and gives repository operations a consistent error boundary.

## Aggregate deletion

`SqliteCascadeRepository` soft-deletes workspace or project descendants in one immediate transaction.
It rechecks existence and running descendants under the write lock so a session or workflow run cannot
become active between validation and updates.

Deleting a worktree task deletes its label and workspace; deleting a project traverses workspaces,
sessions, workflow runs, and worktree records. A running descendant returns `ResourceInUse`; no
partial cascade is committed.

These transactions mutate Ora-owned database state only. They never invoke Git, remove checkout directories or branches, or delete provider-owned ACP history.

SQL details remain internal to this module; lifecycle policy and public error mapping belong to `ora-application` and `ora-backend`.

`SqliteUserConfigRepository` owns the raw `developer_mode`, `log_level`, and
`network_proxy_settings` key/value encodings. Missing rows remain absent and resolve through typed defaults; malformed values fail explicitly, and per-key upserts preserve unrelated preferences.

Repository failures preserve their concrete SQLite errors behind application-owned source-chain wrappers. Skill package promotion and compensation are outside this module, so SQLite never coordinates source copies or filesystem renames. This module does not stringify or log failures that the outer request lifecycle will complete.

See the [ora-db overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts-boundary.md).

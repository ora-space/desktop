# Database Repositories

`ora-db` is the only crate that knows Ora's SQL. It implements the repository ports declared by `ora-application` and hides rows, columns, and soft-delete bookkeeping from every caller above it.

## Implemented ports

| Adapter                                     | Port                                            |
| ------------------------------------------- | ----------------------------------------------- |
| `SqliteProjectRepository`                   | `ProjectRepository`                             |
| `SqliteTaskRepository`                      | `TaskRepository`                                |
| `SqliteSessionRepository`                   | `SessionRepository`                             |
| `SqliteWorktreeRepository`                  | `WorktreeRepository`                            |
| `SqliteSkillRepository`                     | `SkillRepository`                               |
| `SqliteAgentDefinitionRepository`           | `AgentDefinitionRepository`                     |
| `SqliteWorkflowRepository`                  | `WorkflowRepository`                            |
| `SqliteWorkflowRunRepository`               | `WorkflowRunRepository`                         |
| `SqliteCascadeRepository`                   | aggregate deletion used by `ora-backend`        |
| `SqliteTaskWorkspaceRepository`             | `TaskWorkspaceCommit`                           |
| `SqliteUserConfigRepository`                | `UserConfigRepository`                          |
| `SqliteWorktreeProvisioningLeaseRepository` | `WorktreeProvisioningLeaseStore`                |
| `SqliteGitCleanupJobRepository`             | durable Git cleanup queue used by `ora-backend` |

Adding an adapter never changes a port signature. Handlers keep depending on the traits they own, so a composition root can swap in fakes without touching use-case code.

## Connection pool

`DatabaseBootstrapper::bootstrap_repository_pool` reconciles the schema and then returns a cloneable `RepositoryPool`. The pool is `r2d2` over `rusqlite`, and every connection it hands out is configured identically before first use:

- `journal_mode = WAL`
- `busy_timeout = 5000` ms
- `synchronous = NORMAL`

Those PRAGMAs are applied in the connection manager rather than at call sites, so no repository can accidentally run with a different durability or concurrency profile. `NORMAL` is crash-safe for process kills but may lose the last COMMIT on power loss. Skill package promotion flushes directory metadata _before_ that COMMIT, so a lost last transaction leaves an unowned package directory that startup reconciliation removes rather than a visible row without files. Pooling requires a file-backed `DatabaseLocation::Path`; `DatabaseLocation::InMemory` is for bootstrap and migration tests and returns `UnsupportedPooledLocation` if pooled.

File-backed parent directories are not created here. The Desktop composition root prepares them before opening `ora-backend::Backend`.

## Visibility and soft deletion

`is_deleted = 1` is an implementation detail that never leaves the crate.

- `find_*` returns `None` for a soft-deleted row rather than exposing it.
- `list_*` returns only rows whose `is_deleted` flag is unset.
- `soft_delete_*` sets the flag, refreshes `updated_at`, and reports whether one visible entity was affected, which is how handlers distinguish a real delete from a not-found.

`create_*` operations persist the domain value they are given and return what is now stored. Session mutation is intentionally not a full-snapshot replacement: `update_session_title`, `update_session_status`, `update_session_binding`, and `update_session_history_state` each update only the columns owned by that business intent and use `RETURNING` to return the latest complete `Session`. `update_session_title` takes a validated `&SessionTitle`; it cannot clear a title through an ambiguous `Option` argument. The database column remains nullable only for sessions that have not acquired a title. This prevents an actor or connection supervisor holding an older snapshot from overwriting an unrelated title or lifecycle change.

## Row mapping

Repositories map SQLite columns onto the current `ora-domain` shapes, including audit fields and enum-backed columns:

- `worktrees.workspace_id` is both the primary key and the owning Workspace foreign key, so the row cannot acquire an identity independent from its Workspace.
- `sessions.status` becomes `SessionStatus`; `sessions.agent_cli` text becomes `AgentCli` through the namespaced persisted value; nullable `sessions.title` becomes `Option<SessionTitle>` after domain validation; the nullable `sessions.history_degraded_reason` becomes `HistoryState`, where absence means writable.
- `worktrees.is_active` becomes `WorktreeActivity`; `worktrees.branch_name` stays optional.

An unrecognized persisted category value is a mapping failure, not a silently coerced default.

The `user_config` adapter keeps raw SQLite text behind typed application accessors. Missing `developer_mode` and `log_level` rows resolve to `false` and `info`; malformed persisted values fail explicitly instead of being coerced. Writes use canonical lowercase values and update only the requested key.

## Aggregate deletion

`SqliteCascadeRepository` soft-deletes a whole task or project aggregate inside one immediate transaction. It rechecks existence and running descendants under the write lock, so a session cannot transition to `Running` between validation and the update.

- Task deletion cascades through its sessions and its owned worktree record.
- Project deletion cascades through tasks, sessions, and worktree records.
- Both cascades read each worktree-backed task's persisted Git identity (repository root, branch, recorded checkout path) before the soft deletes and insert `git_cleanup_jobs` rows in the same transaction, so physical cleanup intent commits or rolls back atomically with the deletion. Workflow-run deletion (`SqliteWorkflowRunRepository::soft_delete_run`) registers the same jobs for its run-task.

The result is a `CascadeDeleteOutcome` — `Deleted`, `NotFound`, or `ActiveSession` — rather than an error, so the caller decides the public meaning. `ora-backend` maps `ActiveSession` to the stable public code `resource_in_use`, and no partial cascade is committed in that case.

These transactions touch Ora-owned database state only. They never invoke Git, never remove checkout directories or branches, and never delete provider-owned ACP history.

## Error boundary

SQLite execution, query, and row-mapping failures are wrapped in the shared application-owned `RepositoryError`. The wrapper keeps the concrete `DatabaseError` as its `Error::source()` instead of stringifying it, so application and backend layers can add semantic context while diagnostics still reach the original database failure without repeating source text. Database-specific types remain hidden from repository port signatures. Bootstrap and migration failures surface as `DatabaseError`.

Skill import reuses `SqliteSkillRepository` for record reads and writes. Cross-filesystem package promotion, compensation, and per-skill atomicity are application-owned in the filesystem skill storage adapter; SQLite never holds a transaction open while a source archive is copied.

Timestamps used by migration bookkeeping come from an injected `TimestampSource` so tests can be deterministic; `SystemTimestampSource` reads Unix epoch milliseconds from the system clock. Entity `created_at`/`updated_at` values are supplied from above through the application `Clock`, not generated inside the repositories.

See [Application and Contracts Boundary](application-contracts-boundary.md) and [Database Migrations](database-migrations.md).

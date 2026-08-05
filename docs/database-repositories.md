# Database Repositories

`ora-db` is the only crate that knows Ora's SQL. It implements the repository ports declared by `ora-application` and hides rows, columns, and soft-delete bookkeeping from every caller above it.

## Implemented ports

| Adapter | Port |
| --- | --- |
| `SqliteProjectRepository` | `ProjectRepository` |
| `SqliteTaskRepository` | `TaskRepository` |
| `SqliteSessionRepository` | `SessionRepository` |
| `SqliteWorktreeRepository` | `WorktreeRepository` |
| `SqliteSkillRepository` | `SkillRepository` |
| `SqliteAgentDefinitionRepository` | `AgentDefinitionRepository` |
| `SqliteProjectWorkContextRepository` | `ProjectWorkContextRepository` |
| `SqliteTaskDiffCommentRepository` | `TaskDiffCommentRepository` |
| `SqliteWorkflowRepository` | `WorkflowRepository` |
| `SqliteWorkflowRunRepository` | `WorkflowRunRepository` |
| `SqliteCascadeRepository` | aggregate deletion used by `ora-backend` |

Adding an adapter never changes a port signature. Handlers keep depending on the traits they own, so a composition root can swap in fakes without touching use-case code.

## Connection pool

`DatabaseBootstrapper::bootstrap_repository_pool` reconciles the schema and then returns a cloneable `RepositoryPool`. The pool is `r2d2` over `rusqlite`, and every connection it hands out is configured identically before first use:

- `journal_mode = WAL`
- `busy_timeout = 5000` ms
- `synchronous = NORMAL`

Those PRAGMAs are applied in the connection manager rather than at call sites, so no repository can accidentally run with a different durability or concurrency profile. Pooling requires a file-backed `DatabaseLocation::Path`; `DatabaseLocation::InMemory` is for bootstrap and migration tests and returns `UnsupportedPooledLocation` if pooled.

File-backed parent directories are not created here. The composition root prepares them — `ora-backend::Backend::open` does this for both Web and Desktop.

## Visibility and soft deletion

`is_deleted = 1` is an implementation detail that never leaves the crate.

- `find_*` returns `None` for a soft-deleted row rather than exposing it.
- `list_*` returns only rows whose `is_deleted` flag is unset.
- `soft_delete_*` sets the flag, refreshes `updated_at`, and reports whether one visible entity was affected, which is how handlers distinguish a real delete from a not-found.

`create_*` and `update_*` are full-snapshot replacement operations: the repository stores the domain value it was given and returns what is now persisted, without adding transport data or re-deriving fields.

`project_work_contexts` is the exception. It has no `is_deleted` column; expired rows are removed by an explicit delete, and active-ownership queries filter on `lease_expires_at` instead. See [Project Work Contexts](project-work-contexts.md).

## Name-based project lookup

`find_project_by_name` loads one visible project by its exact stored `name` so web-server bootstrap can reconcile its configured workspace identity without listing the whole table. It ignores soft-deleted rows the same way identifier reads do, and returns `None` when only deleted rows match or nothing matches.

## Row mapping

Repositories map SQLite columns onto the current `ora-domain` shapes, including audit fields and enum-backed columns:

- `tasks.status` becomes `TaskStatus`; `tasks.worktree_id` becomes `Option<WorktreeId>`.
- `sessions.status` becomes `SessionStatus`; `sessions.agent_cli` text becomes `AgentCli` through the namespaced persisted value; the nullable `sessions.history_degraded_reason` becomes `HistoryState`, where absence means writable.
- `worktrees.is_active` becomes `WorktreeActivity`; `worktrees.branch_name` stays optional.
- `project_work_contexts.surface` text becomes `ProjectWorkContextSurface`.
- `task_diff_comments` maps root-thread columns and reply columns into the mutually exclusive `TaskDiffCommentKind` enum. Visible comments are returned in `(created_at, id)` order; malformed rows fail rather than being coerced.

An unrecognized persisted category value is a mapping failure, not a silently coerced default.

## Aggregate deletion

`SqliteCascadeRepository` soft-deletes a whole task or project aggregate inside one immediate transaction. It rechecks existence and running descendants under the write lock, so a session cannot transition to `Running` between validation and the update.

- Task deletion cascades through its sessions and its owned worktree record.
- Project deletion cascades through project work contexts, tasks, sessions, and worktree records.

The result is a `CascadeDeleteOutcome` — `Deleted`, `NotFound`, or `ActiveSession` — rather than an error, so the caller decides the public meaning. `ora-backend` maps `ActiveSession` to the stable public code `resource_in_use`, and no partial cascade is committed in that case.

These transactions touch Ora-owned database state only. They never invoke Git, never remove checkout directories or branches, and never delete provider-owned ACP history.

## Error boundary

SQLite execution, query, and row-mapping failures are wrapped in the shared application-owned `RepositoryError`. The wrapper keeps the concrete `DatabaseError` as its `Error::source()` instead of stringifying it, so application and backend layers can add semantic context while diagnostics still reach the original database failure without repeating source text. Database-specific types remain hidden from repository port signatures. Bootstrap and migration failures surface as `DatabaseError`.

Skill import reuses `SqliteSkillRepository` for record reads and writes. Cross-filesystem package promotion, compensation, and per-skill atomicity are application-owned in the filesystem skill storage adapter; SQLite never holds a transaction open while a source archive is copied.

Timestamps used by migration bookkeeping come from an injected `TimestampSource` so tests can be deterministic; `SystemTimestampSource` reads Unix epoch milliseconds from the system clock. Entity `created_at`/`updated_at` values are supplied from above through the application `Clock`, not generated inside the repositories.

See [Application and Contracts Boundary](application-contracts.md) and [Database Migrations](database-migrations.md).

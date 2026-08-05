# Database Migrations

Ora keeps SQLite migration definitions in Rust code inside `ora-db` rather than as standalone `.sql` files. The catalog in `crates/db/src/migration` is the only source of truth for Ora's schema; there is no checked-in `schema.sql`.

## Rules

- Every migration has a unique, strictly increasing version such as `0001`.
- Every migration provides both `up` and `down` statements.
- The `migrations` bookkeeping table stores `version` and `executed_at`, and is created by the first migration alongside the base schema.
- `MigrationCatalog` validates these invariants when it is built, so a duplicate or out-of-order version fails before any statement runs.

## Shipped catalog

| Version | Adds |
| --- | --- |
| `0001` | `projects`, `tasks`, `worktrees`, `virtual_folders`, `virtual_entries`, `sessions`, `artifacts`, `migrations` |
| `0002` | `project_work_contexts` plus its unique `(surface, window_id)` index and lease/expiry indexes |
| `0003` | `skills`, `agents` |
| `0004` | `worktrees.base_commit_id`, `task_diff_comments`, comment indexes, and the root-parent trigger |
| `0005` | `sessions.history_degraded_reason` |
| `0006` | `workflows`, `workflow_snapshots` with a partial unique index for visible `(workflow_id, version)` pairs and foreign key from snapshots to workflows; later extended with `workflow_runs`, `workflow_node_runs`, and `tasks.type`/`workflow_run_id` (unique partial index for the run-task association) |

`default_migration_catalog()` returns all six with every version as the active target.

## Reconciliation model

A catalog carries the full migration list plus an **active target prefix**, which must be a prefix of that list. Requiring a prefix keeps history linear and makes controlled rollback deterministic instead of branch-shaped. `DatabaseBootstrapper::bootstrap` reconciles a database against that target:

- The applied rows in `migrations` are first compared with the target over their shared prefix. Any mismatch is a hard divergence error — migrations are never guessed at, skipped, or reordered.
- If the database is missing trailing target versions, their `up` statements run in ascending order.
- If the database has trailing versions beyond the target prefix, their `down` statements run in reverse order and each rolled-back version is removed from `migrations`.
- An applied version absent from the catalog is an error.
- When the database already matches the target, reconciliation is a no-op.

Each migration's statements and its bookkeeping update run inside **one SQLite transaction**, so a failing statement can never leave the schema and the `migrations` table out of sync, and a failed version is never recorded as applied. Statements execute one at a time so the failing version and direction can be reported precisely.

Because rollback needs `down` statements, retired tail migrations must stay defined in Rust until every managed database has been reconciled to the shorter target prefix.

Migration `0004` is additive for existing databases: it adds the nullable worktree baseline and the new comment table. Its rollback removes only the task-diff indexes, trigger, table, and baseline column; it does not rewrite existing tasks or worktrees. A production rollback must still be treated as destructive for task-diff comments because the down migration drops that table.

Migration `0005` is additive as well: it adds the nullable `sessions.history_degraded_reason` column, and its rollback only drops that column. On-disk conversation history lives outside SQLite, so neither direction touches recorded transcripts.

## Operational logging

`ora-db` emits structured `tracing` events during database bootstrap and reconciliation.

- Database open and bootstrap lifecycle events carry an `operation` field (`database_open`, `database_bootstrap`).
- The reconciliation decision event reports applied and target migration counts plus pending up and down counts; rollback and apply phases log their own counts.
- Migration step events include `migration_version` and `direction`.
- Failures log at `ERROR` with `error.kind` and `error.message` before the original `DatabaseError` is returned to the caller.

The JSON envelope and sink behavior are owned by `ora-logging`; `ora-db` only emits events. See [Runtime Logging](runtime-logging.md) and [Database Repositories](database-repositories.md).

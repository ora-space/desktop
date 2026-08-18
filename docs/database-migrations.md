# Database Migrations

Ora keeps SQLite migration definitions in Rust code inside `ora-db` rather than as standalone `.sql` files. The catalog in `crates/db/src/migration` is the only source of truth for Ora's schema; there is no checked-in `schema.sql`.

## Rules

- Every migration has a unique, strictly increasing version such as `0001`.
- Every migration provides both `up` and `down` statements.
- The `migrations` bookkeeping table stores `version` and `executed_at`, and is created by the first migration alongside the base schema.
- `MigrationCatalog` validates these invariants when it is built, so a duplicate or out-of-order version fails before any statement runs.

## Shipped catalog

| Version | Adds                                                                                                                                                                                       |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `0001`  | Core `projects`, `tasks`, `worktrees`, and `sessions` tables plus migration bookkeeping. Worktree baseline/checkout identity and session title/history state are part of this base schema. |
| `0002`  | `skills` and configurable `agents`, including persisted agent content.                                                                                                                     |
| `0003`  | Constrained `task_diff_comments`, its lookup indexes, and the root-parent trigger.                                                                                                         |
| `0004`  | Workflow definitions, snapshots, runs, node runs, and the task type/workflow-run association.                                                                                              |
| `0005`  | Durable Git cleanup jobs, their dispatch index, and worktree provisioning leases.                                                                                                          |
| `0006`  | Drops unused `tasks.status`.                                                                                                                                                               |

`default_migration_catalog()` returns all migrations with every version as the active target.

## Reconciliation model

A catalog carries the full migration list plus an **active target prefix**, which must be a prefix of that list. Requiring a prefix keeps history linear and makes controlled rollback deterministic instead of branch-shaped. `DatabaseBootstrapper::bootstrap` reconciles a database against that target:

- The applied rows in `migrations` are first compared with the target over their shared prefix. Any mismatch is a hard divergence error — migrations are never guessed at, skipped, or reordered.
- If the database is missing trailing target versions, their `up` statements run in ascending order.
- If the database has trailing versions beyond the target prefix, their `down` statements run in reverse order and each rolled-back version is removed from `migrations`.
- An applied version absent from the catalog is an error.
- When the database already matches the target, reconciliation is a no-op.

Each migration's statements and its bookkeeping update run inside **one SQLite transaction**, so a failing statement can never leave the schema and the `migrations` table out of sync, and a failed version is never recorded as applied. Statements execute one at a time so the failing version and direction can be reported precisely.

The catalog is a clean replacement for the earlier development history. Databases created from that retired history are not supported and must be recreated; the runner compares version identifiers and does not attempt to reinterpret rewritten versions.

Migration `0003` rollback drops all task-diff comments together with their indexes and trigger. Migration `0004` rollback removes workflow execution state before definitions and removes the task association columns in dependency order.

Migration `0005` rollback drops all pending cleanup and provisioning bookkeeping, deliberately re-accepting the pre-migration behavior of leaking physical Git resources on aggregate deletion. The nullable `worktrees.checkout_root` remains part of the base schema because it is worktree identity used by cleanup rather than cleanup-job bookkeeping.

Migration `0006` rollback restores `tasks.status` as an unused integer defaulting to 0.

## Operational logging

`ora-db` emits structured `tracing` events during database bootstrap and reconciliation.

- Database open and bootstrap lifecycle events carry an `operation` field (`database_open`, `database_bootstrap`).
- The reconciliation decision event reports applied and target migration counts plus pending up and down counts; rollback and apply phases log their own counts.
- Migration step events include `migration_version` and `direction`.
- Failures log at `ERROR` with `error.kind` and `error.message` before the original `DatabaseError` is returned to the caller.

The JSON envelope and sink behavior are owned by `ora-logging`; `ora-db` only emits events. See [Runtime Logging](runtime-logging.md) and [Database Repositories](database-repositories.md).

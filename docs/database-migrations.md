# Database Migrations

Ora keeps SQLite migration definitions in Rust code inside `ora-db` rather than as standalone `.sql` files. The catalog in `crates/db/src/migration` is the only source of truth for Ora's schema; there is no checked-in `schema.sql`.

## Rules

- Every migration has a unique, strictly increasing version such as `0001`.
- Every migration provides both `up` and `down` statements.
- The runner creates the `migrations` bookkeeping table with `version`, `up_sql`, `down_sql`, and `executed_at` before loading history.
- Ordered statement lists are trimmed and joined into executable SQL snapshots. Both directions are persisted so either direction changing triggers reconciliation.
- `MigrationCatalog` validates these invariants when it is built, so a duplicate or out-of-order version fails before any statement runs.

## Shipped catalog

| Version | Adds                                                                                                                                                                               |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0001`  | User configuration, projects, workspace locations and provisioning, workspaces, worktrees, task labels, and workspace-owned sessions.                                              |
| `0002`  | Namespaced skills and configurable agents.                                                                                                                                         |
| `0003`  | Workflow definitions, snapshots, workspace-owned runs, and node runs.                                                                                                              |
| `0004`  | Durable Git cleanup jobs and worktree provisioning leases.                                                                                                                         |
| `0005`  | Workspace Effect source state, normalized Desired selections, surface descriptors, ownership ledgers, status, file-operation journals, and durable reconcile/propagation requests. |

`default_migration_catalog()` returns all migrations with every version as the active target.

## Reconciliation model

A catalog carries the full migration list plus an **active target prefix**, which must be a prefix of that list. Requiring a prefix keeps history linear and makes controlled rollback deterministic instead of branch-shaped. `DatabaseBootstrapper::bootstrap` reconciles a database against that target:

- Applied versions are validated against the complete catalog before any mutation. Unknown versions and versions in the wrong position remain hard errors; the runner never guesses, skips, or reorders history.
- Persisted and current `up_sql` and `down_sql` are compared from the beginning of the shared target prefix. `executed_at` is metadata and does not affect equality.
- At the first SQL mismatch, the runner rolls back that migration and the complete applied suffix in reverse order. Rollback always executes the old `down_sql` stored in the database, never the possibly rewritten current definition.
- The runner then applies the current target suffix in ascending order and records fresh SQL snapshots and timestamps.
- If content matches and the database is missing target versions, only the missing tail is applied. If the target is shorter, only the trailing applied versions are rolled back using their stored snapshots.
- When versions and SQL snapshots already match the target, reconciliation is a no-op.

Each migration direction and its bookkeeping update run inside **one SQLite transaction**, so a failing `down` preserves that migration's schema and row, while a failing `up` never records the version. Rebuilding a suffix consists of multiple such steps: if a new `up` fails, already completed rollback steps remain committed and the database stays at that rolled-back prefix.

The catalog is a clean prototype schema organized by logical dependency rather than a compatibility history. It omits retired intermediate tables and columns. Databases whose `migrations` table predates SQL snapshots are unsupported and should be recreated.

Rolling back `0005` removes only Workspace Effect state and durable Effect work; the earlier application schema remains intact.

## Operational logging

`ora-db` emits structured `tracing` events during database bootstrap and reconciliation.

- Database open and bootstrap lifecycle events carry an `operation` field (`database_open`, `database_bootstrap`).
- The reconciliation decision event reports applied and target migration counts plus pending up and down counts; rollback and apply phases log their own counts.
- Migration step events include `migration_version` and `direction`.
- Failures log at `ERROR` with `error.kind` and `error.message` before the original `DatabaseError` is returned to the caller.

The JSON envelope and sink behavior are owned by `ora-logging`; `ora-db` only emits events. See [Runtime Logging](runtime-logging.md) and [Database Repositories](database-repositories.md).

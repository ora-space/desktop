# Database Migrations

Ora keeps SQLite migration definitions in Rust code inside `ora-db` rather than as standalone `.sql` files. The catalog in `crates/db/src/migration` is the only source of truth for Ora's schema; there is no checked-in `schema.sql`.

## Rules

- Every migration has a unique, strictly increasing version such as `0001`.
- Every migration provides both `up` and `down` statements.
- The runner creates the `migrations` bookkeeping table with `version`, `up_sql`, `down_sql`, and `executed_at` before loading history.
- Ordered statement lists are trimmed and joined into executable SQL snapshots. Both directions are persisted so explicit development reconciliation can detect either direction changing.
- `MigrationCatalog` validates these invariants when it is built, so a duplicate or out-of-order version fails before any statement runs.
- A catalog may carry first-install SQL outside the versioned snapshots. The public default is an
  empty statement list; an internal distribution can patch that list without rewriting migration
  history.

## Shipped catalog

| Version | Adds                                                                                                                                                                                                           |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0001`  | User configuration, projects, workspace locations and provisioning, workspaces, worktrees, task labels, and workspace-owned sessions.                                                                          |
| `0002`  | Namespaced skills and configurable agents.                                                                                                                                                                     |
| `0003`  | Workflow definitions, snapshots, workspace-owned runs, and node runs.                                                                                                                                          |
| `0004`  | Durable Git cleanup jobs and worktree provisioning leases.                                                                                                                                                     |
| `0005`  | Durable plugin marketplace source configuration.                                                                                                                                                               |
| `0006`  | Generic Effect Scopes, Sources/Revisions, Desired State, Consumers/Targets, shared Resources, projections, ownership, statuses, Conditions, claims, Attempts, operation journals, receipts, and audit history. |
| `0007`  | Immutable marketplace-source namespace bindings keyed by canonical Git URL.                                                                                                                                    |
| `0008`  | Per-source `enabled` flag so a marketplace URL can be disabled without deleting its identity.                                                                                                                  |
| `0009`  | Source-scoped tagged artifact retrieval configuration with Direct HTTPS as the migration default.                                                                                                              |

`default_migration_catalog()` returns all migrations with every version as the active target and
the empty public first-install SQL list.

## Application startup

`DatabaseBootstrapper` validates that the database and target have an identical shared version
prefix. It applies a missing target tail in ascending order. If the database contains versions
introduced by a newer application, it rolls that trailing suffix back in reverse order using the
`down_sql` stored by the newer version. It does not compare persisted SQL snapshots for shared
versions, so packaged application startup cannot rebuild user data merely because an old migration
definition changed.

When the database had no migration rows before bootstrap and at least one target migration was
applied, the runner executes first-install SQL after the target is complete. Those statements run
in one separate transaction and are not recorded in `migrations`. Existing databases never receive
them later. If their transaction fails, bootstrap fails while the schema migrations that already
committed remain applied.

## Development reconciliation

A catalog carries the full migration list plus an **active target prefix**, which must be a prefix of that list. Requiring a prefix keeps history linear and makes controlled rollback deterministic instead of branch-shaped. The explicit `reconcile_migration_history` tooling interface reconciles a database against that target:

- Applied versions in the shared target prefix are validated before any mutation. Unknown versions and versions in the wrong position within that prefix remain hard errors; versions in a trailing applied suffix are rollback input and need not exist in the current catalog.
- Persisted and current `up_sql` and `down_sql` are compared from the beginning of the shared target prefix. `executed_at` is metadata and does not affect equality.
- At the first SQL mismatch, the runner rolls back that migration and the complete applied suffix in reverse order. Rollback always executes the old `down_sql` stored in the database, never the possibly rewritten current definition.
- The runner then applies the current target suffix in ascending order and records fresh SQL snapshots and timestamps.
- If content matches and the database is missing target versions, only the missing tail is applied. If the target is shorter, only the trailing applied versions are rolled back using their stored snapshots.
- When versions and SQL snapshots already match the target, reconciliation is a no-op.

Explicit development reconciliation follows the same first-install rule for a database with no
applied migration history.

`cargo xtask reconcile-migrations DATA_DIRECTORY` invokes this interface. `task run:desktop` runs
that command against the repository `.data` directory immediately before starting Tauri, keeping
automatic rollback and migration-rewrite support confined to local development.

Each migration direction and its bookkeeping update run inside **one SQLite transaction**, so a failing `down` preserves that migration's schema and row, while a failing `up` never records the version. Rebuilding a suffix consists of multiple such steps: if a new `up` fails, already completed rollback steps remain committed and the database stays at that rolled-back prefix.

The catalog is a clean prototype schema organized by logical dependency rather than a compatibility history. It omits retired intermediate tables and columns. Databases whose `migrations` table predates SQL snapshots are unsupported and should be recreated.

Rolling back `0006` removes only Effect state and durable Effect work; the earlier application and
plugin marketplace schema remains intact.

## Operational logging

`ora-db` emits structured events during database bootstrap and explicit reconciliation.

- Database open and bootstrap lifecycle events carry an `operation` field (`database_open`, `database_bootstrap`).
- Application bootstrap and explicit reconciliation report applied and target migration counts plus pending `up` and `down` counts; rollback and apply phases log their own counts.
- Migration step events include `migration_version` and `direction`.
- Failures log at `ERROR` with `error.kind` and `error.message` before the original `DatabaseError` is returned to the caller.

The JSON envelope and sink behavior are owned by `ora-logging`; `ora-db` only emits events. See [Runtime Logging](runtime-logging.md) and [Database Repositories](database-repositories.md).

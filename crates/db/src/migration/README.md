# Database Migration Module

This module owns Ora's linear, reversible SQLite schema history and reconciles a database to an explicit target prefix.

## Catalog invariants

- `MigrationCatalog` requires unique, strictly increasing versions.
- The active target must be a prefix of the complete catalog. This makes controlled rollback deterministic and rejects branch-shaped histories.
- Every migration contains ordered up and down statements. The default catalog includes every shipped schema migration.
- Skills, configurable agents, and workflows use `(namespace, name)` as their case-insensitive
  visible identity. Soft-deleted rows do not reserve that identity, and local resources use the
  `local` namespace.
- Applied versions are recorded in the `migrations` table with an injected execution timestamp.

## Reconciliation

`reconcile_database` first verifies that the applied history matches the target over their shared prefix. A mismatch is a hard divergence error; migrations are never guessed, skipped, or reordered.

Trailing applied migrations are rolled back in reverse order. Pending migrations are applied in forward order. Each migration step and its bookkeeping update run in one SQLite transaction, so a failing statement cannot leave the schema and migration table out of sync.

An applied version absent from the catalog is an error. Reconciliation is otherwise idempotent when the database already matches the target.

Schema contents stay in version-specific modules; repository query behavior belongs to the repository module. The compressed catalog has seven boundaries:

- `0001` installs the core project, task, worktree, session, and bookkeeping schema.
- `0002` installs skills and configurable agents.
- `0003` installs constrained task-diff comments, indexes, and the root-only parent trigger.
- `0004` installs workflow definitions, snapshots, execution records, and task associations.
- `0005` installs durable Git cleanup jobs and worktree provisioning leases.
- `0006` drops unused `tasks.status`.
- `0007` installs durable plugin eligibility keyed only by filesystem-derived plugin id.

The catalog intentionally replaces the retired development history rather than providing a compatibility bridge. Databases created from the old history must be recreated. Rollback of `0003` or `0005` discards the corresponding comments or cleanup bookkeeping even though each step remains transactional.

See the [ora-db overview](../../README.md) and [Database Migrations](../../../../docs/database-migrations.md).

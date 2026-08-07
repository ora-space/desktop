# Database Migration Module

This module owns Ora's linear, reversible SQLite schema history and reconciles a database to an explicit target prefix.

## Catalog invariants

- `MigrationCatalog` requires unique, strictly increasing versions.
- The active target must be a prefix of the complete catalog. This makes controlled rollback deterministic and rejects branch-shaped histories.
- Every migration contains ordered up and down statements. The default catalog includes every shipped schema migration.
- Applied versions are recorded in the `migrations` table with an injected execution timestamp.

## Reconciliation

`reconcile_database` first verifies that the applied history matches the target over their shared prefix. A mismatch is a hard divergence error; migrations are never guessed, skipped, or reordered.

Trailing applied migrations are rolled back in reverse order. Pending migrations are applied in forward order. Each migration step and its bookkeeping update run in one SQLite transaction, so a failing statement cannot leave the schema and migration table out of sync.

An applied version absent from the catalog is an error. Reconciliation is otherwise idempotent when the database already matches the target.

Schema contents stay in version-specific modules; repository query behavior belongs to the repository module. Migration `0004` adds the nullable worktree baseline plus the constrained task-diff comment table, indexes, and root-only parent trigger. Its down statements deliberately remove all task-diff comment data, so rollback is an operationally destructive action even though reconciliation remains transactional. Migration `0005` adds the nullable `sessions.history_degraded_reason` column. Migration `0006` adds workflow definitions, snapshots, runs, node runs, and the task type/workflow-run association columns. Migration `0007` adds audited, soft-deleted project specification source overrides with workflow/custom-name constraints and an active-path unique index. Migration `0008` adds the persisted content column to configurable agents. Migration `0009` adds the nullable persisted `sessions.title` column; acquisition and lock state remain runtime-only.

See the [ora-db overview](../../README.md) and [Database Migrations](../../../../docs/database-migrations.md).

# Database Migration Module

This module owns Ora's linear, reversible SQLite schema history and reconciles a database to an explicit target prefix. Its public interface is the validated catalog plus `reconcile_database`; SQL snapshot comparison and suffix rebuilding remain internal.

## Catalog invariants

- `MigrationCatalog` requires unique, strictly increasing versions.
- The active target must be a prefix of the complete catalog. This makes controlled rollback deterministic and rejects branch-shaped histories.
- Every migration contains ordered up and down statements. Their trimmed, joined SQL is the stable executable snapshot used for comparison and rollback.
- The default catalog contains six dependency-ordered modules: workspace core, Agent/Skill catalog, workflows, Git lifecycle bookkeeping, application configuration, and Workspace Effect state.
- Skills, configurable agents, and workflows use `(namespace, name)` as their case-insensitive
  visible identity. Soft-deleted rows do not reserve that identity, and local resources use the
  `local` namespace.
- Applied rows record `version`, `up_sql`, `down_sql`, and an injected `executed_at` timestamp.
- Migration `0006` is the Effect v2 model: stable Sources, immutable Revisions and explicit Heads;
  normalized Workspace Desired items; Surface and Consumer declarations/status; Managed ownership;
  current Conditions; durable reconcile/propagation requests; mutation Operations and recovery
  Artifacts; and append-only Audit events.
- Every Workspace has exactly one Effect aggregate. The first release installs every active Skill
  Source by default: publishing a new local or plugin Skill adds it to all existing Workspaces, and
  the Workspace insert trigger selects all active Skill Heads for a newly created Workspace.
- Local Skill Sources use namespace `local`. Plugin Skill Sources use the owning plugin's canonical
  `<plugin_namespace>/<plugin_identifier>` identity. Plugin enabled state is deliberately outside
  the Effect selection policy.

## Reconciliation

`reconcile_database` first verifies that every applied version belongs to the catalog and occupies the expected position. Unknown, skipped, or reordered versions are hard errors.

It then compares the persisted SQL snapshots with current migration definitions over the shared target prefix. At the first changed `up_sql` or `down_sql`, it rolls back the complete applied suffix in reverse order using each row's persisted `down_sql`, then applies the current target suffix in forward order and stores fresh snapshots. Timestamps do not participate in this comparison.

Ordinary target shortening uses the same persisted rollback snapshots. Ordinary target growth applies only the missing tail. Each migration step and its bookkeeping update run in one SQLite transaction, so a failing statement cannot leave that step's schema and row out of sync. Earlier successful rollback steps remain committed if a later rewritten `up` fails.

An applied version absent from the catalog is an error. Reconciliation is otherwise idempotent when the database already matches the target.

The prototype catalog describes only the current schema. It does not carry migrations for retired tables or columns, and the bookkeeping table is intentionally not compatible with databases created before SQL snapshots were introduced. Development databases may be recreated; no compatibility bridge is provided.

Rolling back `0006` removes only Workspace Effect state and durable Effect work, leaving the earlier workspace, catalog, workflow, Git lifecycle, and plugin schemas intact.

See the [ora-db overview](../../README.md) and [Database Migrations](../../../../docs/database-migrations.md).

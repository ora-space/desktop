# Database Migration Module

This module owns Ora's linear, reversible SQLite schema history. Application bootstrap aligns migration versions without comparing SQL snapshots. Explicit development tooling may additionally reconcile a database to an exact target prefix based on SQL snapshot comparison and suffix rebuilding. Concrete schema definitions live in the private [`schema`](schema/) module, which is the single migration registry consumed by the catalog.

## Catalog invariants

- `MigrationCatalog` requires unique, strictly increasing versions.
- The active target must be a prefix of the complete catalog. This makes controlled rollback deterministic and rejects branch-shaped histories.
- Every migration contains ordered up and down statements. Their trimmed, joined SQL is the stable executable snapshot used for comparison and rollback.
- The default catalog contains ten dependency-ordered modules: workspace core and application
  configuration, Agent/Skill catalog, workflows, Git lifecycle bookkeeping, marketplace source
  configuration, Generic Effect persistence, immutable marketplace-source namespace bindings,
  marketplace enabled flags, artifact retrieval configuration, and independent Effect audit time.
- Skills, configurable agents, and workflows use `(namespace, name)` as their case-insensitive
  visible identity. Soft-deleted rows do not reserve that identity, and local resources use the
  `local` namespace.
- Applied rows record `version`, `up_sql`, `down_sql`, and an injected `executed_at` timestamp.
- The default catalog carries a separate first-install SQL statement list. Public builds keep the
  list empty; distribution patches may populate it without changing any versioned migration
  snapshot. Its contents are omitted from `MigrationCatalog` debug output.
- Migration `0005` stores plugin marketplace source configuration.
- Migration `0006` stores Generic Effect Scopes, Sources/Revisions, complete Desired State,
  Consumer/Target declarations, shared Resources, projections, ownership, statuses, Conditions,
  fenced requests/claims, Attempts, operation journals, receipts, and audit history.
- Migration `0007` records the namespace bound to each marketplace source's canonical URL. It is a
  table of its own rather than a column on the source row because the binding outlives the
  configuration: once a plugin from that source is installed, the namespace is frozen into its
  install path, private data directory, `skills` rows, and Effect Consumer identity, so deleting
  the source must leave the binding behind and re-adding the repository must reuse it. Nothing
  updates or deletes a row — writing a second identity for one repository would strand every row
  the first identity owns.
- Migration `0008` adds an `enabled` flag to marketplace sources. Disabling a source keeps its URL,
  branch, proxy policy, position, and namespace binding, but drops it from marketplace sync,
  listing, and install until it is enabled again.
- Migration `0009` adds the tagged `artifact_retrieval` JSON configuration. Existing sources
  default to Direct HTTPS; the S3 SigV4 variant keeps endpoint, bucket, region, and the credential
  pair together instead of spreading them across nullable columns.
- Migration `0010` separates recovery `detected_at` from row `updated_at` and removes the Scope
  insert trigger. Project and task Workspace repositories create and seed the Scope within their
  write transaction using an injected audit clock. Existing Effect rows and authority are retained.
  Rollback restores recovery detection to the old column and reinstalls the Workspace trigger.
- Target requests use pending, claimed, blocked, and retry-scheduled states. Generation and fencing
  establish authority; audit time never grants a claim or changes retry eligibility.
- Every Workspace has one Scope. Publishing a new Skill Source seeds existing Scopes; creating a
  Workspace seeds its new Scope from published Sources in the same transaction.

## Application bootstrap

Application bootstrap validates the version sequence shared by the database and current target.
It applies a missing target tail in ascending order. If the database contains a trailing suffix
introduced by a newer application, bootstrap executes that suffix's persisted `down_sql` in
reverse order and removes its bookkeeping rows. It does not compare persisted SQL snapshots for
shared versions, so changing a previously published migration definition cannot trigger a schema
rebuild during packaged application startup.

After a database with no prior migration rows reaches its target, application bootstrap executes
the catalog's first-install SQL in one separate transaction. An existing database never runs that
SQL, including when a later build first supplies a non-empty list. The SQL is data initialization,
not migration history: it has no migration row, does not participate in drift comparison, and is
not rolled back with a target suffix. If initialization fails, the already committed schema
migrations remain applied and bootstrap returns the failure.

## Explicit development reconciliation

`reconcile_database` first verifies that the applied and target histories have an identical shared version prefix. Unknown, skipped, or reordered versions inside that prefix are hard errors; an applied suffix beyond the target is rollback input and does not need to exist in the current catalog.

It then compares the persisted SQL snapshots with current migration definitions over the shared target prefix. At the first changed `up_sql` or `down_sql`, it rolls back the complete applied suffix in reverse order using each row's persisted `down_sql`, then applies the current target suffix in forward order and stores fresh snapshots. Timestamps do not participate in this comparison.

Ordinary target shortening uses the same persisted rollback snapshots. Ordinary target growth applies only the missing tail. Each migration step and its bookkeeping update run in one SQLite transaction, so a failing statement cannot leave that step's schema and row out of sync. Earlier successful rollback steps remain committed if a later rewritten `up` fails.

An applied version absent from the catalog inside the shared target prefix is an error. Reconciliation is otherwise idempotent when the database already matches the target.

The public `reconcile_migration_history` interface opens and reconciles a database for explicit
tooling. `cargo xtask reconcile-migrations DATA_DIRECTORY` calls it before `task run:desktop`
starts the application; packaged application startup never calls this interface.

Explicit reconciliation uses the same first-install rule when it opens a database with no applied
migration history.

The prototype catalog describes only the current schema. It does not carry migrations for retired tables or columns, and the bookkeeping table is intentionally not compatible with databases created before SQL snapshots were introduced. Development databases may be recreated; no compatibility bridge is provided.

Rolling back `0006` removes only Generic Effect state and durable Effect work, leaving the earlier workspace, catalog, workflow, and Git lifecycle schemas intact.

See the [ora-db overview](../../README.md) and [Database Migrations](../../../../docs/database-migrations.md).

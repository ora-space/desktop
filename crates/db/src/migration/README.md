# Database Migration Module

This module owns Ora's linear, reversible SQLite schema history. Application bootstrap aligns migration versions without comparing SQL snapshots. Explicit development tooling may additionally reconcile a database to an exact target prefix based on SQL snapshot comparison and suffix rebuilding. Concrete schema definitions live in the private [`schema`](schema/) module, which is the single migration registry consumed by the catalog.

## Catalog invariants

- `MigrationCatalog` requires unique, strictly increasing versions.
- The active target must be a prefix of the complete catalog. This makes controlled rollback deterministic and rejects branch-shaped histories.
- Every migration contains ordered up and down statements. Their trimmed, joined SQL is the stable executable snapshot used for comparison and rollback.
- The default catalog contains eleven dependency-ordered modules: workspace core and application
  configuration, Agent/Skill catalog, workflows, Git lifecycle bookkeeping, marketplace source
  configuration, Generic Effect persistence, immutable marketplace-source namespace bindings,
  marketplace enabled flags, artifact retrieval configuration, independent Effect audit time,
  and workflow execution scopes.
- Skills, configurable agents, and workflows use `(namespace, name)` as their case-insensitive
  visible identity. Soft-deleted rows do not reserve that identity, and local resources use the
  `local` namespace.
- Applied rows record `version`, `up_sql`, `down_sql`, and an injected `executed_at` timestamp.
- `bootstrap_state` records fresh-install provenance atomically with creation of migration
  bookkeeping, before schema steps can commit. Pending first-install data survives failures and
  its completion commits with that data. Marketplace initialization is independent of distributor SQL.
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
- Migration `0011` persists the MCP selection owned by each Session. All existing Sessions
  default to an empty explicit set because MCP authorization has not yet been used by users;
  migration does not inspect workflow metadata. New Sessions explicitly persist their selection,
  and rollback removes the column.
- Target requests use pending, claimed, blocked, and retry-scheduled states. Generation and fencing
  establish authority; audit time never grants a claim or changes retry eligibility.
- Migration `0012` adds foreach iteration indices to node runs.
- Migration `0013` adds workflow root/round identities and scoped node uniqueness. Its downgrade
  preserves scope/node evidence in an append-only archive while terminalizing active Loop runs;
  the archive survives re-upgrade. See [execution scope storage](../../../../docs/workflows/loops-and-iterations.md).
- The unpublished Loop branch previously used `0011`. Startup recognizes its exact SQL snapshot,
  installs upstream `0011`/`0012`, and adopts it as `0013` atomically, preserving live scope history.
- Migration `0022` records the marketplace import state of each workflow document, keyed by
  `(source namespace, plugin identifier, document path)`. An `imported` row is committed in the same
  transaction that creates the workflow, its draft, and its first published snapshot, and it
  deliberately carries no foreign key to `workflows`: copy liveness is resolved on read against
  `workflows.is_deleted = 0`, so a surviving copy keeps its source identity across upgrades,
  renames, and reinstalls, a copy deleted during an install cycle stays deleted for upgrades and
  card retries while a fresh marketplace install reconsiders it (and conditionally deletes the
  row back to unrecorded if recovery fails for a non-conflict reason), and `conflict_skipped` stays
  re-checkable on every install, upgrade, and card retry. A non-conflict import failure on an
  unrecorded or `conflict_skipped` path leaves that state unchanged, keeping it retryable.
- Migration `0019` adds the `finalized_without_effect` operation phase used by the explicit Effect
  recovery exit. Its upgrade rebuilds the operation journal and its referencing status/artifact
  tables through staging copies so every row and foreign key survives; the downgrade maps closed
  journals back to explicit `recovery_required`, which a later upgrade can prove and close again.
- Migration `0024` is data only: it marks local Main Workspaces that earlier project creation left
  in `provisioning` (and their `pending` `local_git` provisioning rows) as `active`/`ready`, since
  nothing ever promotes a local checkout. Remote locations and deleted rows are untouched. Its
  downgrade is intentionally empty because older builds admit an active local Workspace the same
  way.
- Migration `0025` adds nullable `package_root` onto `skills`, backfills plugin Skill roots from
  retired Effect source revisions, and drops all remaining `effect_*` triggers and tables in
  child-before-parent order. Its persisted downgrade explicitly aborts inside the rollback
  transaction because the retired ledger and filesystem ownership markers cannot be restored, so an
  older build refuses to strip the `0025` bookkeeping row and a newer build can reopen cleanly.
  - **Automatic pre-`0025` backup**: before `0025` runs on an existing pre-`0025` database, both
    packaged Desktop bootstrap (`Backend::open`) and `cargo xtask reconcile-migrations` write a
    self-contained SQLite backup into a sibling temporary file (`VACUUM INTO`) and atomically
    replace `ora.sqlite3.pre-0025.bak` in the same data directory. Every startup while the legacy
    `effect_*` tables still exist refreshes that backup (so a zero-byte leftover or an earlier
    downgrade-and-reupgrade cycle never leaves a stale copy), while fresh and already-migrated
    databases skip backup creation.
  - **Downgrade recovery**: upgrading past `0025` cannot be downgraded in place. To return to an
    older Desktop release, quit Ora Desktop, remove `ora.sqlite3`, `ora.sqlite3-wal`, and
    `ora.sqlite3-shm` from the data directory, copy `ora.sqlite3.pre-0025.bak` to `ora.sqlite3`,
    and launch the older build.
  - **Frozen SQL text**: `0025`'s `up_sql` and `down_sql` are frozen once merged (see below).

## Application bootstrap

Application bootstrap validates the version sequence shared by the database and current target.
It applies a missing target tail in ascending order. If the database contains a trailing suffix
introduced by a newer application, bootstrap executes that suffix's persisted `down_sql` in
reverse order and removes its bookkeeping rows. It does not compare persisted SQL snapshots for
shared versions, so changing a previously published migration definition cannot trigger a schema
rebuild during packaged application startup.

Fresh database provenance and pending first-install work are recorded atomically before schema
steps. After reaching the target, bootstrap runs pending catalog first-install SQL and clears its
journal in one transaction. A restart resumes this work even if migrations were already committed;
an older database without that journal never runs new distribution defaults. The SQL is data initialization,
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

**Irreversible boundary at `0025`:** development databases that have already applied migration
`0025` cannot reconcile or roll back past `0025` because `0025`'s persisted `down_sql` aborts the
rollback transaction. Consequently, `0025`'s SQL text (`up_sql` and `down_sql`) — as well as every
earlier migration's SQL snapshot — is frozen for post-`0025` development databases: editing `0025`
(or an earlier migration) causes `reconcile_database` to detect snapshot drift at or before `0025`,
attempt to roll back `0025`, and fail. Subsequent schema changes must be added as new migrations
(`0026+`), or the development database must be recreated (or restored from
`ora.sqlite3.pre-0025.bak`).

Explicit reconciliation also resumes first-install work from the durable journal.

The prototype catalog describes only the current schema. It does not carry migrations for retired tables or columns, and the bookkeeping table is intentionally not compatible with databases created before SQL snapshots were introduced. Development databases may be recreated; no compatibility bridge is provided.

Rolling back `0006` removes only Generic Effect state and durable Effect work, leaving the earlier workspace, catalog, workflow, and Git lifecycle schemas intact.

Migration `0026` records evidence for new workflow attempts and run-local debug operations. Downgrading removes these records; upgrading does not infer historical inputs. Connections that insert node runs or write debug evidence must register the `ora_debug_hash` SQL function as the application does. A stock `sqlite3` connection cannot perform those writes.

See the [ora-db overview](../../README.md) and [Database Migrations](../../../../docs/core/persistence.md).

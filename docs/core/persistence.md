# Persistence

English | [中文](persistence.zh.md)

Desktop keeps its application state in one SQLite database owned by `ora-db`. `ora-db` is the only
crate that knows Ora's SQL: it defines the migration catalog, bootstraps the database and
implements the repository ports declared by `ora-application` and `ora-effect`. Callers above it
never see rows, columns or soft-delete bookkeeping. The Node and the process host keep their own,
separate databases; see [Node](../node/node.md) and [Process host](../process/host.md).

## Migration catalog

Migrations are Rust code in `crates/db/src/migration`, not standalone `.sql` files, and the catalog
is the only source of truth for the schema; there is no checked-in `schema.sql`.

- Every migration has a unique, strictly increasing version (`0001`) and ordered `up` and `down`
  statement lists. Their trimmed, joined SQL is the snapshot persisted after execution.
- `MigrationCatalog` validates these invariants when it is built, so a duplicate or out-of-order
  version fails before any statement runs.
- The `migrations` bookkeeping table stores `version`, `up_sql`, `down_sql` and `executed_at`.
- Each migration direction and its bookkeeping update run in one SQLite transaction.
- The catalog carries an optional first-install SQL list outside versioned history. Public builds
  keep it empty; a distribution can patch it without rewriting migrations.
- Shipped migrations are append-only. A migration that lands after another branch took its number
  is renumbered before merging; startup recognizes one historical case (an unpublished Loop branch
  that used `0011`) by its exact SQL and adopts it as `0013`.

| Version | Adds                                                                                                                             |
| ------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `0001`  | User configuration, projects, Workspace locations and provisioning, Workspaces, worktrees, task labels, workspace-owned sessions |
| `0002`  | Namespaced Skills and configurable agents                                                                                        |
| `0003`  | Workflow definitions, snapshots, workspace-owned runs and node runs                                                              |
| `0004`  | Durable Git cleanup jobs and worktree provisioning leases                                                                        |
| `0005`  | Plugin marketplace source configuration                                                                                          |
| `0006`  | Generic Effect persistence, see [Effects](../agents/effects.md)                                                                  |
| `0007`  | Immutable namespace binding per marketplace source, keyed by canonical Git URL                                                   |
| `0008`  | Per-source `enabled` flag                                                                                                        |
| `0009`  | Per-source artifact retrieval configuration (Direct HTTPS by default)                                                            |
| `0010`  | Independent Effect recovery detection time; Scope creation moves into Workspace transactions                                     |
| `0011`  | Each Session's persisted MCP selection (existing sessions get an empty explicit set)                                             |
| `0012`  | Iteration round index on node runs                                                                                               |
| `0013`  | Workflow execution scopes: root and Loop-round identities, scoped node uniqueness, downgrade archive                             |
| `0014`  | Pack installation ownership (`pack_installation`, `pack_installation_member`)                                                    |
| `0015`  | Scheduled tasks, scheduled runs and occurrences                                                                                  |
| `0016`  | Scheduled task scheduling anchor and worktree base branch                                                                        |
| `0017`  | Scheduled run worktree base branch selected at admission                                                                         |
| `0018`  | Scheduled run worktree task link                                                                                                 |
| `0019`  | The `finalized_without_effect` Effect operation phase, added by rebuilding the operation journal through staging copies          |
| `0020`  | Scheduled run business verdict, detail and annotation time                                                                       |
| `0021`  | Scheduled run artifact manifest and rejected-candidate log                                                                       |
| `0022`  | Releases local Main Workspaces stranded in `provisioning` (data only; downgrade is a no-op)                                      |

Details of individual migrations live in [`crates/db/src/migration/README.md`](../../crates/db/src/migration/README.md).
The catalog describes only the current schema; databases whose `migrations` table predates SQL
snapshots are unsupported and must be recreated.

## Application startup

`DatabaseBootstrapper` checks that the database and the target share an identical version prefix,
applies a missing tail in ascending order, and rolls back a trailing suffix introduced by a newer
application in reverse order using the `down_sql` that newer version stored. It never compares the
SQL of shared versions, so a packaged build cannot rebuild user data because an old migration
definition changed.

When the database had no migration rows and at least one migration was applied, the first-install
SQL runs afterwards in its own transaction and is not recorded in `migrations`. Existing databases
never receive it. If it fails, bootstrap fails while the committed migrations stay applied.

## Development reconciliation

`reconcile_migration_history` is the explicit tooling path for SQL drift during development. The
catalog carries the full migration list plus an active target that must be a prefix of it, which
keeps history linear and rollback deterministic.

- Versions in the shared prefix are validated first; unknown or misplaced versions are hard errors.
  An applied suffix beyond the target is rollback input and need not exist in the catalog.
- Persisted and current `up_sql`/`down_sql` are compared from the start of the prefix
  (`executed_at` is ignored). At the first mismatch the runner rolls back that migration and the
  whole applied suffix in reverse order, always with the stored `down_sql`, then applies the current
  suffix and records fresh snapshots.
- Matching content with missing versions applies only the tail; a shorter target only rolls back.
  A database that already matches is a no-op.
- If a rewritten `up` fails, the rollback steps already completed stay committed.

`cargo xtask reconcile-migrations DATA_DIRECTORY` invokes this interface, and `task run:desktop`
runs it against the repository `.data` directory before starting Tauri, so rollback and rewrite
support stays confined to local development.

## Connections

`DatabaseBootstrapper::bootstrap_repository_pool` reconciles the schema and returns a cloneable
`RepositoryPool` (`r2d2` over `rusqlite`). Every connection is configured identically in the
connection manager, so no repository can run with a different profile:

- `journal_mode = WAL`
- `busy_timeout = 5000` ms
- `synchronous = FULL`

`FULL` is required for power-loss durability: in WAL mode `NORMAL` appends COMMIT frames without an
fsync, so a power cut can roll the database back by a whole session. `FULL` fsyncs the WAL on every
commit, so at most the in-flight transaction is lost. Skill package promotion flushes directory
metadata before that commit, so a lost last transaction leaves an unowned package directory that
startup reconciliation removes, never a visible row without files.

Pooling requires a file-backed `DatabaseLocation::Path`; `DatabaseLocation::InMemory` is for
bootstrap and migration tests. Parent directories are prepared by the Desktop composition root.

## Repositories

| Adapter                                                                                                                | Port or role                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `SqliteProjectRepository`, `SqliteTaskRepository`, `SqliteWorkspaceRepository`, `SqliteWorktreeRepository`             | Project, task, Workspace and worktree records                                                                |
| `SqliteTaskWorkspaceRepository`                                                                                        | The single-transaction commit of a new task, see [Tasks and worktrees](../workspaces/tasks-and-worktrees.md) |
| `SqliteWorktreeProvisioningLeaseRepository`, `SqliteGitCleanupJobRepository`                                           | Provisioning leases and the durable Git cleanup queue                                                        |
| `SqliteCascadeRepository`                                                                                              | Aggregate deletion used by `ora-backend`                                                                     |
| `SqliteSessionRepository`                                                                                              | Sessions                                                                                                     |
| `SqliteSkillRepository`, `SqliteAgentDefinitionRepository`                                                             | Skill and agent catalogs                                                                                     |
| `SqliteWorkflowRepository`, `SqliteWorkflowRunRepository`, `SqliteWorkflowRunEngineRepository`                         | Workflow definitions, run CRUD and the run engine's transactions                                             |
| `SqliteScheduledTaskRepository`, `SqliteScheduledRunRepository`                                                        | Scheduled tasks and their runs                                                                               |
| `SqlitePluginMarketplaceSourceRepository`, `SqlitePluginSourceNamespaceRepository`, `SqlitePackInstallationRepository` | Marketplace sources, namespace bindings and pack ownership                                                   |
| `SqliteEffectRepository`                                                                                               | Effect state, see [Effects](../agents/effects.md)                                                            |
| `SqliteUserConfigRepository`                                                                                           | The generic key/value store for user configuration                                                           |

Adding an adapter never changes a port signature.

### Visibility and soft deletion

`is_deleted = 1` never leaves the crate. `find_*` returns `None` for a soft-deleted row, `list_*`
returns visible rows only, and `soft_delete_*` sets the flag, refreshes `updated_at` and reports
whether one visible entity was affected, which is how handlers tell a delete from a not-found.

Session mutation is not a full-snapshot replacement: `update_session_title`,
`update_session_status`, `update_session_binding` and `update_session_history_state` each update
only the columns that business intent owns and return the latest complete row with `RETURNING`, so
an actor holding an older snapshot cannot overwrite an unrelated change.

### Row mapping

Repositories map columns onto the current domain shapes, including enum-backed columns. An
unrecognized persisted category is a mapping failure, never a silently coerced default.
`worktrees.workspace_id` is both the primary key and the owning Workspace foreign key, so a
worktree cannot acquire an identity independent of its Workspace.

The `user_config` adapter implements the generic raw key/value port from `ora-user-config` and
owns only reads, upserts and deletes. Typed interpretation stays above it: the application layer
owns developer mode, log level and network proxy defaults and validation, and the Backend owns the
`worktree_root` path policy. Malformed persisted values fail explicitly, and each write updates only
the requested key.

### Aggregate deletion

`SqliteCascadeRepository` soft-deletes a whole task or project aggregate inside one immediate
transaction and rechecks existence and running descendants under the write lock, so a session
cannot become `Running` between validation and update. The cascade reads each worktree-backed
task's Git identity (repository root, branch, recorded checkout path) and inserts
`git_cleanup_jobs` rows in the same transaction; workflow-run deletion registers the same jobs. The
result is a `CascadeDeleteOutcome` (`Deleted`, `NotFound` or `ActiveSession`); `ora-backend` maps
`ActiveSession` to `resource_in_use`. These transactions never invoke Git and never delete
provider-owned ACP history.

### Errors and time

SQLite execution, query and mapping failures are wrapped in the application-owned
`RepositoryError`, which keeps the concrete `DatabaseError` as its `Error::source()` instead of
stringifying it. Bootstrap and migration failures surface as `DatabaseError`.

Migration bookkeeping takes timestamps from an injected `TimestampSource`. Entity `created_at` and
`updated_at` come from the application `Clock`. Effect repositories sample their own
`TimestampSource` after acquiring the write transaction, see [Effects](../agents/effects.md#audit-and-business-time).

## Operational logging

Bootstrap and reconciliation emit structured events: open and bootstrap events carry an
`operation` (`database_open`, `database_bootstrap`), reports include applied, target and pending
counts, and each migration step carries `migration_version` and `direction`. Failures log at
`ERROR` before the original `DatabaseError` is returned. See [Logging](logging.md).

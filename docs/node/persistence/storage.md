# Node local storage

English | [中文](storage.zh.md)

`NodeConfig.home_directory` is an explicitly injected absolute path; there is no HOME-derived default.
The database is always `home_directory/ora-node.sqlite3`. Managed deployment requires an owner-private
directory distinct from host state; existing permissions are never silently changed.

This filename remains unchanged when combined with the process subsystem. Node owns
business execution, resource and event records; the host and guardian own their separate journals.
The OS lock excludes another Node database owner, but is not proof that an older Git process stopped.
The workspace's bundled SQLite version is shared; no older engine or host schema is imported.

Opening a database holds an exclusive OS file lock for its lifetime. SQLite uses its default
rollback journal and FULL synchronous writes. A new database receives application ID `0x4f52414e`
and schema version 3. Exact version 1/2 schemas migrate transactionally after identity and integrity
validation, preserving executions, results and pending events. Existing empty files, foreign databases, unsupported versions, directories
and corrupt databases are rejected without rebuilding them. The persistent NodeId survives
reopening; each Node runtime generates a fresh NodeIncarnationId. An explicit identity mismatch
fails initialization.

`ora-node-db` owns `node_metadata`, `executions`, `resources`, `outbox`, `managed_executions` and `process_attempts`.
Version 3 adds `execution_identities`, `clone_executions`, `clone_outbox` and `process_outcomes`.
The common identity table prevents clone/Worktree key collisions. Process associations now reference
that table; migration copies every original association without rewriting RunSpec or inventing outcomes.
Old version-2 binaries reject version 3 before migration; downgrading does not reset or recreate this file.
Clone persistence is described in [repository acquisition](repository-acquisition.md).
The complete command is stored separately from the resolved target, which freezes the canonical
binding, authorized roots, task path, branch and base commit. Unique operation/execution identities
prevent rebinding. Active resources reserve workspace, path and repository-local branch before Git
runs, including overlapping reserved paths. Deletion references existing ownership and retains a tombstone after completion.

Before mutation dispatch, the process journal records the execution, original host directory/UID,
Scope/Run identities and exact RunSpec as bounded MessagePack. A narrow second SQLite connection shares
the original OS lease; it cannot outlive that lease or become another Node authority. Cleanup acknowledgement
retains historical associations. Failure to write an association prevents dispatch; failure to acknowledge
cleanup requires querying the original Scope again. `CleanupCreation` persists branch-only recovery intent.

Migration does not invent process evidence for legacy direct-Git executions. An old in-flight execution
without managed association stays blocked; only an untouched Accepted execution can enter managed execution.
The database lock alone never authorizes cleanup. Environment values in RunSpec are private data, not logs.

Guarded transitions preserve Accepted, Running, Unknown and Completed evidence. Completion commits
resource facts, the terminal result and its original event envelope together. Status reads have no
acknowledgement effect. An acknowledgement must match Node, operation, execution and sequence 1;
it removes only the delivery record. Results and execution deduplication survive acknowledgement.

`WriteGuard` exposes transaction failure points for real SQLite fault tests. Tests in `ora-node-db`
cover exclusive ownership, file preservation, deduplication, reservations, rollback, reopening and
acknowledgement. Run `cargo test -p ora-node-db -p ora-node`.

Opening also checks table/index definitions and foreign-key integrity; the schema identifier alone
cannot authorize an unknown structure. A definitive no-effect Worktree creation failure retires reservations
and releases active uniqueness while retaining execution deduplication and the failed result.
Inconclusive executions continue to hold their reservations.

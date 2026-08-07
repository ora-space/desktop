# ora-backend

`ora-backend` is the transport-neutral composition root shared by Web and Tauri adapters. It opens persistent state, wires concrete application repositories and handlers, supervises agent providers, and exposes one stable `Backend` API over contract DTOs.

## Responsibilities

- `Backend::open` creates required directories, bootstraps and migrates SQLite, reconciles imported skill packages, constructs APIs, and starts the [agent runtime](src/agent_runtime/README.md).
- `Backend::open` also owns one `AppEventHub`. It exposes the hub through the transport adapters as a best-effort invalidation stream and injects only its internal publisher into session actors; the hub does not depend on Axum or Tauri.
- The shared `ora-scheduler::Scheduler` owns actor-facing delayed work. Scheduler tasks enqueue internal commands, while actors remain the only code that calls ACP or writes session state.
- Project, task, skill CRUD, atomic skill-folder import, and agent operations delegate to `ora-application`; aggregate deletion uses transactional database cascades.
- `TaskDiffApi` composes the task-diff handlers with SQLite and Gitlancer. It resolves the agent's live task cwd, uses `HEAD` as the moving baseline for project-root tasks, and uses the persisted creation commit for isolated worktrees.
- `SpecApi` composes target resolution, project-wide source overrides, bounded ripgrep discovery, safe Markdown reads, and watcher-root resolution. Web and Tauri remain transport-only adapters.
- Task diff reads, commits, pushes, and comments preserve the same public error projection as the rest of the backend. Git and SQLite sources remain internal diagnostics and are rendered once by the adapter-owned request lifecycle.
- Session creation, loading, structured ACP prompting, permissions, stopping, deletion, and model discovery delegate to the agent runtime. Creation also returns the provider's setup-time available-command catalog.
- `BackendError` retains the internal source chain while exhaustively projecting semantic failures into a typed `PublicError` and one transport-neutral `ErrorClassification`. HTTP derives status from the classification; all adapters serialize the same direct `ContractError`.
- `RequestLifecycle` gives Web, Tauri, and stream seams one generated request id and an exactly-once success, failure, or cancellation completion event. Failure log levels derive from `ErrorClassification`.
- The configured worktree root affects only task creations that begin after an update. Existing task paths are resolved from persisted worktree identity and Git's authoritative metadata.

## Ownership boundaries

Project and task deletion soft-delete Ora-owned database records in one transaction and reject aggregates with running sessions. These paths do not call Git and do not delete provider-owned ACP history.

`ProjectWorkContext` and general-purpose filesystem browsing remain outside this crate. Specification filesystem access is composed here because it combines persisted project configuration with target ownership. Logging initialization and environment parsing belong to runtime composition roots. This crate provides the transport-neutral request lifecycle, while adapters decide where a request begins and completes.

Dropping the last backend owner shuts down provider supervisors and initiates bounded process-tree cleanup.

The application event stream is deliberately not an event log: events are not persisted or replayed, a bounded queue may terminate a slow subscription, and clients refetch the database-backed queries after stream loss. One active application client instance owns the stream at a time; the ownership check is performed when the App Shell starts rather than on every backend operation.

See [Application and Contracts Boundary](../../docs/application-contracts.md) and [ACP Agent Runtime](../../docs/agent-runtime.md).
See also [Specification management](../../docs/spec-management.md).

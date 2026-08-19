# ora-backend

`ora-backend` is the Desktop composition root behind the Tauri adapter. It opens persistent state, wires concrete application repositories and handlers, supervises agent providers, and exposes one stable `Backend` API over contract DTOs.

## Responsibilities

- `Backend::open` creates required directories, bootstraps and migrates SQLite, reconciles imported skill packages (catalog rows whose on-disk package is missing or unreadable stay unavailable instead of refusing to start), constructs APIs, starts the [agent runtime](src/agent_runtime/README.md), and composes the workflow run engine.
- Workflow run start, restart, HITL, and cancel are live production paths: `build_workflow_run_engine` constructs `WorkflowRunEngine` with `WorkflowRunNodeExecutor` as the `NodeExecutor`. The executor drives each agent node through a real Ora session and reports completion through `WorkflowRunCallback`. Adapters call the resulting `WorkflowRunControlHandler`; they do not construct the executor.
- `PluginApi` composes `ora-plugin-lifecycle` with SQLite, the bundled Deno runtime, and one `AppEventHub`. `Backend::open` scans plugin identity at startup without auto-activation; later scans and lifecycle actions flow through the stable `Backend` API.
- `Backend::open` exposes the event hub through transport adapters as a best-effort invalidation stream and injects only its internal publisher into stateful components; the hub does not depend on Axum or Tauri.
- The shared `ora-scheduler::Scheduler` owns actor-facing delayed work. Scheduler tasks enqueue internal commands, while actors remain the only code that calls ACP or writes session state.
- Project, task, skill CRUD, atomic skill-folder import, and agent operations delegate to `ora-application`; aggregate deletion uses transactional database cascades.
- `TaskDiffApi` composes the task-diff handlers with SQLite and Gitlancer. It resolves the agent's live task cwd, uses `HEAD` as the moving baseline for project-root tasks, and uses the persisted creation commit for isolated worktrees.
- `SpecApi` composes target resolution, automatic bounded ripgrep discovery, safe Markdown reads, and watcher-root resolution. Tauri remains a transport-only adapter.
- Task diff reads, commits, pushes, and comments preserve the same public error projection as the rest of the backend. Git and SQLite sources remain internal diagnostics and are rendered once by the adapter-owned request lifecycle.
- Session creation, loading, structured ACP prompting, permissions, stopping, deletion, and model discovery delegate to the agent runtime. Creation also returns the provider's setup-time available-command catalog.
- Relative project roots are resolved against a bootstrap-injected path base, not live process cwd. Desktop `tauri dev` starts in `src-tauri`; a shared `ORA_DATA_DIR` database stores roots relative to that data directory's parent.
- `BackendError` retains the internal source chain while exhaustively projecting semantic failures into a typed `PublicError` and one transport-neutral `ErrorClassification`. Tauri commands and channels serialize the same direct `ContractError`.
- `RequestLifecycle` gives Tauri command and stream seams one generated request id and an exactly-once success, failure, or cancellation completion event. Failure log levels derive from `ErrorClassification`. Dropping the last handle without an explicit completion records an `abandoned` outcome, so the one-completion-per-request invariant holds structurally rather than by convention.
- The configured worktree root affects only task creations that begin after an update. Existing task paths are resolved from persisted worktree identity and Git's authoritative metadata.

## Ownership boundaries

Graph parsing, DAG scheduling, and node-run state belong to `ora-application`'s [workflow run engine](../application/src/workflow_run/engine/README.md). This crate owns the session-driving `WorkflowRunNodeExecutor` and the composition that attaches `WorkflowRunCallback` before serving commands. That executor is a live production path, not a test-only stub.

Project, task, and workflow-run deletion soft-delete Ora-owned database records in one transaction, reject aggregates with running sessions, and register durable Git cleanup jobs in that same transaction. The crate-owned `git_cleanup` worker executes those jobs asynchronously — force-removing each deleted task's linked worktree and `ora/*` branch with at-least-once, idempotent semantics, replaying pending jobs and expired provisioning leases on every start. Deletion never touches provider-owned ACP history, and cleanup that cannot prove Ora ownership parks as `manual_attention` instead of removing the checkout.

General-purpose filesystem browsing remains outside this crate. Specification filesystem access is composed here because it combines persisted project configuration with target ownership. Logging initialization and environment parsing belong to runtime composition roots. This crate provides the transport-neutral request lifecycle, while adapters decide where a request begins and completes.

Dropping the last backend owner shuts down provider supervisors and initiates bounded process-tree cleanup.

The application event stream is deliberately not an event log: events are not persisted or replayed, a bounded queue may terminate a slow subscription, and the Desktop shell refetches database-backed queries after stream loss. Every active channel may subscribe to the same broadcast. Adapters that abort consumption may use `SessionEventStream::try_recv` to observe a buffered terminal error without waiting for the next event.

See [Application and Contracts Boundary](../../docs/application-contracts-boundary.md), [ACP Agent Runtime](../../docs/agent-runtime.md), and [Workflow](../../docs/workflow.md).
See also [Specification management](../../docs/spec-management.md).

# ora-backend

`ora-backend` is the Desktop composition root behind the Tauri adapter. It opens persistent state, wires concrete application repositories and handlers, supervises agent providers, and exposes domain interfaces through `Backend`.

`Backend::agents()`, `skills()`, and `workflows()` return shared handles for configurable-agent
definitions, Skill catalog/import, and workflow definitions/drafts/versions. They expose contract
DTO use cases and `BackendError`, not repositories, constructors, or execution engines. Adding
a use case in one of these modules does not add a root forwarding method. Skill mutations still
own their Effect wakeups, and definition use cases cannot accidentally start workflow execution.
Desktop command execution captures only the relevant handle; surface download actions also use
the Skill interface without retaining the entire Backend.

`Backend::projects()` and `tasks()` own their complete aggregate use cases. Deletion holds the
same transactional active-session/workflow checks, registers durable Git cleanup, removes the
recorded histories, and only then wakes cleanup. The async deletion receiver shares the existing
handle into the blocking repository task; callers never issue these steps separately. Task
construction receives one named, crate-private `TaskSetup`, retaining the same provisioning gates
used by cleanup. Workspace Git operations keep their shared use leases. Backend does not expose
its repository pool.

`Backend::workspaces()` is a cloneable handle for visible workspace queries, authoritative local
path resolution, worktree-root configuration, and Git diff/commit/push. Clones share the original
root lock and cleanup use-lease registry. Raw persisted-root absence remains an internal bootstrap
concern, not a public use case. Native file browsing/search/watch implementations stay in Desktop
and `ora-fs`; they inject only this workspace handle alongside their file reader.

`Backend::plugins()` returns `Plugins`, the public interface in `plugin/operations.rs`. It owns
scan/import/install/update/uninstall and the required follow-up agent-set reconciliation, while
the existing private `PluginApi` remains the shared host implementation used by runtime, Effect,
configuration, and gateway adapters. This keeps lifecycle, registry identity, generation leases,
and marketplace source rules unchanged without exposing those internals to Desktop. New public
coordination belongs in the operations module instead of extending the already-large host file.

`Backend::workflow_runs()` owns run persistence, scheduling control, manual completion, and
post-commit session cleanup. Desktop captures this handle; the callback and manual control paths
share the same run gates. Completion claims fence follow-up prompts until a commit or preparation
failure, and cancellation is revalidated after preparation. Crash recovery and baseline pruning
live in `workflow/run/recovery.rs`; startup still runs them before accepting commands.

`Backend::sessions()` returns `Sessions`, which owns persisted queries, title edits plus actor
adoption/invalidation, and session runtime use cases. It receives only `WorkflowSessionTurns`
from workflow-run composition, not scheduling control or exposed locks. That restricted interface
owns prompt admission, failed-start restoration, and stream-drop cleanup using the same run gates
and completion claims. `Backend::app_events()` exposes the shared subscriber source; publication
remains crate-private. Consumers do not retain the whole Backend to run a session use case.

`Backend::agent_runtime()` offers only readiness and on-demand model discovery, and
`Backend::effects()` offers persisted target status from the worker's shared pool. Their private
implementations retain the existing supervisor/reconciliation ownership. Stateless Git identity
resolution is an explicitly exported function, not a method that requires the entire Backend.
The root now contains path/bootstrap handling and handle accessors; adding an operation in an
existing domain does not change it. No legacy root-operation forwarding interface remains.

## Responsibilities

- `Backend::open` creates required directories, bootstraps and migrates SQLite, reconciles imported
  skill packages and their active Effect source revisions (catalog rows whose on-disk package is
  missing or unreadable stay unavailable instead of refusing to start), constructs APIs, starts
  the [agent runtime](src/agent_runtime/README.md), and composes the workflow run engine.
- Workflow run start, restart, HITL, and cancel are live production paths: `build_workflow_run_engine` constructs `WorkflowRunEngine` with `WorkflowRunNodeExecutor` as the `NodeExecutor`. The executor drives each agent node through a real Ora session and reports completion through `WorkflowRunCallback`. Run creation validates enabled skills through an Agent delivery-capability provider and freezes the invocation names and Effect-owned package paths without copying packages. Before prompting, the executor builds a structured handoff containing those materialized paths, the current task, resolved role constraints, full topological node/status panorama, original run request, and each successful predecessor's final assistant output in execution order. Generated connective copy uses the Ora display locale frozen when the run was created, while user-authored content remains unchanged; leading skill slash commands remain first for CLI parsing. Adapters call `WorkflowRuns`; its private control handler and executor are never exposed.
- `PluginApi` composes `ora-plugin-lifecycle` and `ora-plugin-config` with SQLite, the bundled Deno runtime, one `AppEventHub`, and a `PluginNotificationSink` with two delivery paths: a lossy broadcast whose receivers (`PluginGateway::subscribe_notifications`) let the desktop surface host route best-effort notifications such as `ora/ui/push` without blocking the lifecycle pump, and lossless per-generation taps for consumers that cannot drop a frame. It is the only owner of plugin processes and of the transport-facing Plugin Configuration editor: `Backend::open` discovers installed packages once under `BackendPaths::home_directory` without auto-activation, the agent runtime reaches an agent plugin only through `PluginApi::attach_agent` (a `PluginConnection` plus a tap of that generation), later scans and lifecycle actions flow through `Backend::plugins()`, and revision-checked configuration changes stay host-owned rather than being injected into Agent processes. Skill plugins additionally sync catalog projections through `SqliteSkillRepository` when packages are installed, imported, scanned, or uninstalled. Marketplace catalog sync pulls every **enabled** SQLite-backed source, merges their indexes with first-source-wins identity, and writes one cached `registry_index.json`; disabled sources stay persisted (including their namespace binding) but are skipped for sync, listing, and install. A source marked `use_proxy` applies the user-configured proxy to its Git fetch. Plugin install and update honor the same per-source policy and send the archive download through the configured Reqwest proxy when enabled. Host-level proxy settings can be cleared, and Settings can probe an arbitrary HTTP(S) URL through the form's proxy without persisting it. Targeted Hook releases are selected against the current host triple before download. Plugin install and update return a typed `InstallOutcome`: a conflict-free install yields `installed`; a Hook whose command alias collides with another installed Hook yields `installed_with_command_conflict` carrying the colliding identity. Both packages remain available; uniqueness is deferred to a future consumer. An installed valid Hook is globally available with a `stopped` runtime and no separate enablement state.
- `backend.plugins().gateway()` returns the `PluginGateway` the desktop surface layer drives: installed-package lookup, the plugin's writable data directory, on-demand process connections (`ensure_running` / `connection`), idle stop, and the `SurfaceCloser` injection point.
- `Backend::open` exposes the event hub through transport adapters as a best-effort invalidation stream and injects only its internal publisher into stateful components; the hub does not depend on Axum or Tauri.
- The shared `ora-scheduler::Scheduler` owns actor-facing delayed work. Scheduler tasks enqueue internal commands, while actors remain the only code that calls ACP or writes session state.
- Project, task, skill CRUD, atomic skill-folder import, and agent operations delegate to `ora-application`; aggregate deletion uses transactional database cascades.
- `Backend::settings()` exposes the cloneable `Settings` interface for developer mode, preferred
  log level, and network proxy use cases. Runtime logging receives only
  `settings.preferred_log_level_store()`. Settings hides construction, repositories, raw keys,
  and worktree configuration; Desktop never receives the whole runtime just to execute a preference
  operation. Async preference calls dispatch SQLite work to the blocking pool. Internal synchronous
  proxy reads also serve plugin retrieval policy. The settings tests open only SQLite and exercise
  persistence/reopening and injected storage faults through this interface.
- `WorkspaceApi` composes the workspace-diff handlers with SQLite and Gitlancer, keyed by `WorkspaceId` for either an isolated task worktree or a project's main checkout. It resolves the workspace's live cwd and, when a `Worktree` row is recorded for it, uses the persisted creation commit as the stable diff baseline; a workspace with no such row has no baseline (only the `Unstaged`/`Staged` scopes apply) and its writes go through unverified.
- Tauri remains a transport-only adapter.
- Workspace diff reads, commits, and pushes preserve the same public error projection as the rest of the backend. Git and SQLite sources remain internal diagnostics and are rendered once by the adapter-owned request lifecycle.
- Session creation, loading, structured ACP prompting, permissions, stopping, deletion, and model discovery delegate to the agent runtime. Creation also returns the provider's setup-time available-command catalog. Every `session/new` and `session/load` shares one Session MCP Snapshot; see [Session MCP](../../docs/session-mcp.md).
- Relative local Workspace locations are resolved against a bootstrap-injected path base, not live process cwd. Desktop `tauri dev` starts in `src-tauri`; a shared `ORA_DATA_DIR` database stores locations relative to that data directory's parent.
- `BackendError` retains the internal source chain while exhaustively projecting semantic failures into a typed `PublicError` and one transport-neutral `ErrorClassification`. Tauri commands and channels serialize the same direct `ContractError`.
- `RequestLifecycle` gives Tauri command and stream seams one generated request id and an exactly-once success, failure, or cancellation completion event. Failure log levels derive from `ErrorClassification`. Dropping the last handle without an explicit completion records an `abandoned` outcome, so the one-completion-per-request invariant holds structurally rather than by convention.
- The configured worktree root affects only isolated Workspace creations that begin after an update. Existing task paths are resolved from persisted Workspace locations and Git's authoritative metadata.

## Ownership boundaries

Graph parsing, DAG scheduling, and node-run state belong to `ora-application`'s [workflow run engine](../application/src/workflow_run/engine/README.md). This crate owns the session-driving `WorkflowRunNodeExecutor` and the composition that attaches `WorkflowRunCallback` before serving commands. That executor is a live production path, not a test-only stub.

Project and task deletion soft-delete their Workspace-owned descendants in one transaction and register durable Git cleanup jobs in that same transaction. Workflow-run deletion only cascades the run, node runs, and sessions bound to those node runs; it preserves the shared Workspace. The crate-owned `git_cleanup` worker executes task cleanup asynchronously — force-removing each deleted task's linked worktree and `ora/*` branch with at-least-once, idempotent semantics, replaying pending jobs and expired provisioning leases on every start. Deletion never touches provider-owned ACP history, and cleanup that cannot prove Ora ownership parks as `manual_attention` instead of removing the checkout.

General-purpose filesystem browsing remains outside this crate. Logging initialization and environment parsing belong to runtime composition roots. This crate provides the transport-neutral request lifecycle, while adapters decide where a request begins and completes.

Dropping the last backend owner shuts down provider supervisors and initiates bounded process-tree cleanup.

The application event stream is deliberately not an event log: events are not persisted or replayed, a bounded queue may terminate a slow subscription, and the Desktop shell refetches database-backed queries after stream loss. Every active channel may subscribe to the same broadcast. Adapters that abort consumption may use `SessionEventStream::try_recv` to observe a buffered terminal error without waiting for the next event.

See the [backend workflow module](src/workflow/README.md), [Application and Contracts Boundary](../../docs/application-contracts-boundary.md), [ACP Agent Runtime](../../docs/agent-runtime.md), and [Workflow](../../docs/workflow.md).

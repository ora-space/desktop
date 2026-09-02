# Workflow

`ora-application` owns the workflow definition use cases, with persistence in `ora-db` and public contracts in `ora-contracts`. Workflows manage editable agent orchestration graphs with draft-as-workspace semantics and immutable published snapshots.

## Entities and tables

| Domain type        | Backing table        |
| ------------------ | -------------------- |
| `Workflow`         | `workflows`          |
| `WorkflowSnapshot` | `workflow_snapshots` |

`Workflow` holds the stable identity (name, published snapshot pointer, audit fields) while `WorkflowSnapshot` owns the versioned React Flow graph. Read models (`WorkflowDetail`, `WorkflowSummary`, `WorkflowVersion`) keep graph data out of list responses. The library list is newest-created first so a just-created workflow is the first row.

## Draft, publish, and version lifecycle

Every workflow has exactly one `draft` snapshot created atomically with the workflow itself. The draft is an editable workspace: `UpdateDraft` mutates its graph in-place without creating a new snapshot row.

The settings library can duplicate a workflow by creating a new identity and draft from the source's current draft graph. A duplicate never carries published snapshots, the active published pointer, or workflow runs; it starts as an editable, unpublished workflow.

Publishing copies the draft's graph into a new, immutable snapshot. `updated_at` on published snapshots is `NULL`, including after soft deletion — only draft editing changes that field. Publish always activates the new snapshot (sets `workflows.published_snapshot_id`), making it the version used by any future workflow execution.

Additional operations keep the version model flexible without data loss:

- **Rollback** copies any historical snapshot's graph into the draft. It does not change the published pointer, so the active version stays unchanged while the editor workspace resets to a known state.
- **Activate** switches the published pointer to a different snapshot and syncs its graph into the draft. This is the explicit "make this version live" operation for cases where publish-and-activate-together is not desired.
- **Snapshot deletion** removes individual published snapshots but refuses to delete the draft or the currently active version.

## Identifiers and versioning

`WorkflowId` and `WorkflowSnapshotId` are UUID-backed newtypes following the same `define_id!` macro convention as every other domain entity.

Snapshot versions are strings. The draft is identified by the reserved string `"draft"`. Published versions can be user-provided (e.g. `"v1.0.0"`) or automatically derived as `v{timestamp_millis}` from the same injected application clock that supplies `created_at`. If an automatic version collides in the same millisecond, a numeric suffix is added without changing its clock-derived prefix. User-provided versions must be non-blank, at most 128 bytes, safe for a single URL path segment, and cannot be `"."` or `".."`. A partial unique index prevents duplicate visible versions within one workflow, so a soft-deleted version name may be reused.

## Graph storage

The `graph` column stores the complete React Flow JSON document. Workflow definition CRUD treats it as an opaque string. The [workflow run engine](../crates/application/src/workflow_run/engine/README.md) parses and validates the frozen snapshot when a run starts.

## Handlers

The `workflow` module exposes the full set of CRUD and lifecycle handlers, all following the existing port-adapter pattern with `WorkflowRepository`, `WorkflowIdGenerator`, and `Clock`:

| Handler                   | Purpose                                         |
| ------------------------- | ----------------------------------------------- |
| `CreateWorkflowHandler`   | Create workflow with initial draft              |
| `GetWorkflowHandler`      | Fetch workflow + draft + published snapshot     |
| `ListWorkflowsHandler`    | List visible workflows without graph data       |
| `UpdateWorkflowHandler`   | Rename workflow                                 |
| `DeleteWorkflowHandler`   | Soft-delete workflow with cascade               |
| `GetDraftHandler`         | Fetch draft snapshot with graph                 |
| `UpdateDraftHandler`      | Mutate draft graph in-place                     |
| `PublishWorkflowHandler`  | Freeze draft as immutable snapshot and activate |
| `RollbackWorkflowHandler` | Copy historical graph into draft                |
| `ActivateWorkflowHandler` | Switch published pointer and sync draft         |
| `ListVersionsHandler`     | List published version summaries                |
| `GetVersionHandler`       | Fetch a specific snapshot by version string     |
| `DeleteSnapshotHandler`   | Soft-delete a published snapshot (constrained)  |

Unlike project and task, workflow deletion follows the standard CRUD handler pattern rather than a separate cascade repository, because the deletion constraints are simpler (no running-session check).

## Workflow runs

A workflow run freezes one published snapshot and executes directly in a caller-selected Workspace. A project's row targets its Main Workspace; a Task row targets that Task's Isolated Workspace. The run CRUD layer is graph-agnostic. The execution engine owns start/restart/HITL on top of the same repository; `ora-backend` implements `NodeExecutor` as `WorkflowRunNodeExecutor` and composes it at `Backend::open`.

Before an agent node is prompted, the backend turns the frozen graph and current node-run rows into a structured workflow handoff. The message identifies the current step and its direct neighbors, labels the resolved role as behavioral constraints, lists every node in deterministic topological order with its current status, separates the original run request from predecessor output, and includes the final assistant message from each successful transitive predecessor in that same execution order. Ora's active display locale is frozen when the run is created, so every generated handoff in that run uses consistent Chinese or English copy even if the interface language later changes. User-authored node content and predecessor output remain verbatim. Enabled skill slash commands remain at the beginning of the first text block because agent CLIs parse them positionally. That block also lists the actual skill-package paths in the selected Workspace and requires the Agent to use those materialized copies.

Skill delivery is capability-driven. Effect owns physical Skill materialization in the Workspace. During run creation, the backend asks an `AgentSkillDeliveryProvider` for each skill-using node's validated, Workspace-relative discovery roots and freezes a per-node receipt containing the original skill id, executable slash-command name, and actual package paths; it does not copy or rewrite packages. The current provider returns the shared `.agents/skills` root for every Agent. A future plugin-backed provider may return different or multiple roots without changing workflow creation, executor, or prompt-rendering code. Node execution consumes only the frozen receipt and never re-resolves skill names from the mutable global catalog; capability or catalog changes therefore affect new runs only.

The session history is the sole source of a node's complete conversation. `workflow_node_runs.output` stores only that node's final assistant text, which is the scalar handoff consumed by downstream nodes and the run output calculation. A node Session remains addressable by id for Theater, but standalone Session listing excludes every Session bound to a visible workflow node run so workflow execution never appears as an ordinary chat.

### Entities and tables

| Domain type       | Backing table        |
| ----------------- | -------------------- |
| `WorkflowRun`     | `workflow_runs`      |
| `WorkflowNodeRun` | `workflow_node_runs` |

`WorkflowRun` pins `snapshot_id` to the user-released version it was created against and stores its own display name and `workspace_id`. `WorkflowNodeRun` records one executed node; nodes that never started have no row, and the frontend derives "not started" by comparing graph nodes against recorded node runs. Run and node status share the same five-value enums (`Pending | Running | Succeeded | Failed | Cancelled`). An interactive node parked awaiting follow-up input is persisted as `Pending`; the public contract derives a `Running` run with an awaiting node as `AwaitingInput` so the sidebar can surface that human action is needed. A session bound to a terminal node is read-only: the backend rejects new prompts against it.

### Creation and snapshot pinning

`CreateWorkflowRunHandler` admits the requested active `workspace_id`, resolves the frozen snapshot — an explicit `snapshot_id` or the workflow's `published_snapshot_id` — validates that it belongs to the requested workflow together with the declared roles and Skill bindings, and persists the run. Runs start `Pending` with an empty `current_nodes` anchor. When the caller supplies no kickoff input, the run inherits the frozen snapshot's Start node instruction as its default input. The frontend obtains the exact Workspace from the project or Task row that opened the workflow picker; the backend does not infer a branch or create another worktree.

### Reading and deleting

Get returns the run with its display name and node runs; list returns project-scoped summaries. Node-run history is read-only in this layer — the engine owns node-run writes and the state machine.

Deletion refuses active runs — a `Running` run, a HITL pause with a non-terminal node run, or a `Running` session bound to one of the run's nodes — and then soft-deletes the run, its node runs, and node-owned sessions. A not-started `Pending` run (empty `current_nodes`, no node rows) can be discarded without cancelling. The selected Workspace is shared infrastructure and is never deleted with one run. Soft-deleted runs are invisible to queries and cannot be reactivated.

### Snapshot protection

A published snapshot referenced by a live run cannot be soft-deleted (`SnapshotInUse`), and deleting a workflow whose snapshots a live run freezes is refused (`ActiveRuns`), so a run's frozen graph stays readable across its lifecycle.

## Boundaries (non-goals)

- Workflow-run CRUD persists runs and node-run records but does not execute them. Graph execution, node-run writes, and the state machine (start/restart/HITL) are engine-owned in `ora-application`, with agent-node sessions driven by `ora-backend`'s `WorkflowRunNodeExecutor`. Runtime variables and checkpoints are out of scope for this layer.
- Graph validation at run start belongs to the workflow run engine, not the definition CRUD handlers.
- Tauri command registration and web-server route wiring are transport concerns owned by the respective adapters.

See [Domain Models](domain-models.md), [Application and Contracts Boundary](application-contracts-boundary.md), [Database Repositories](database-repositories.md), [Workflow Run Engine](../crates/application/src/workflow_run/engine/README.md), [ora-backend](../crates/backend/README.md).

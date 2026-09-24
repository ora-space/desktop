# Workflow

English | [中文](workflow.zh.md)

Delivered extension: [Workflow Loop implementation plan and evidence](workflow-loop-plan.md).

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

The `graph` column stores the complete React Flow JSON document. Workflow definition CRUD treats it as an opaque string; the [workflow run engine](../crates/application/src/workflow_run/engine/README.md) parses and validates the frozen snapshot when a run starts.

## Nodes excluded from execution

Authors can retain spare nodes and connected groups outside the execution path. Drafts, published snapshots, rollback, and import/export preserve the complete canvas; execution derives an entry-reachable subgraph from the frozen snapshot without rewriting it.
Root reachability starts at Start; active Loops use their child Start, and active Iterations use their entry edges. Every member of an unused container is excluded.
Static analysis follows every Condition outlet; runtime branch selection remains distinct from static exclusion.

Spare nodes and their edges never enter scheduling, variable pools, or role and Skill preparation, and create no NodeRun or Session. An edge from a spare node into an active node is excluded so it cannot block a join. Active nodes referencing spare outputs fail validation before execution. Reconnecting a node restores normal execution validation.
Document-wide checks still reject duplicate IDs, dangling edges, invalid ownership, and illegal cross-scope edges; configuration and executability checks apply to the execution subgraph.

The editor and run overview use backend analysis to show “Excluded from execution”; the editor also shows a count. Overview retains spare nodes, while Theater excludes them from its execution path; clicking a spare node does not open a waiting-to-execute view. Membership is derived from topology rather than a persisted node switch. Analysis results are bound to document identity, and switching documents or unmounting cancels obsolete requests.

## Loop containers

Executable Loop graphs use `schemaVersion: 2`. A root Loop owns `data.loopConfig`; every child
declares the same Loop through both React Flow `parentId` and `data.containerId`. The editor creates
a valid container group with one child Start, one child Agent, and their internal edge. Root and
child connections cannot cross scope boundaries, deleting a Loop removes its descendants and
incident edges atomically, and root auto-layout preserves child-relative positions. Both the
editor and run Overview render owned nodes inside the Loop body. Editor children remain bounded
by that body, and authors can resize the selected Loop; the saved dimensions are reused by
published snapshots and run Overview.

`loopConfig` defines a 1–100 round bound, typed carried variables, simultaneous feedback selectors,
a typed `until` condition, and named exports. Each Loop body is a separate DAG with exactly one
reachable Start. Nested Loops, ownership mismatches, cross-scope edges and selectors, invalid
types on active nodes are rejected before sessions start. Spare children unreachable from the child Start remain in the snapshot and do not execute. The default editor group feeds
the child Agent output into the next round's `value`, stops on a non-empty output, and exports it as
`result`; authors can set the initial value and maximum rounds.

Each iteration has a durable `WorkflowExecutionScope`. Child NodeRuns and Sessions belong to that
scope, so repeated definition node IDs do not overwrite another round. Completion resolves feedback
and termination from the completed pool, then commits either the next scope or the parent Loop
result in one repository transaction. Cancellation and child failure settle the active scope and
parent together. Restart rotates the root execution identity, preserving old history while making
late callbacks harmless. The real run contract returns ordered scope identities, and the Theater
Loop inspector lets users switch rounds and inspect each round's child status and session ID.

## Agent-node MCP bindings

The Agent inspector reads installed `kind: "mcp"` plugins, displaying their names, canonical IDs,
and configuration availability. Authors can add, enable, disable, and remove bindings independently
for each node. Adding enables the binding. Loading failures offer retry and preserve existing
bindings; missing plugins remain visible by ID and can still be disabled or removed.
Installing an MCP adds it to this global authoring catalog only; MCP is never materialized into
Workspace files, and the enabled bindings form the node Session's strict allowlist.

Bindings use `mcps: [{ mcpId, enabled }]` in the graph, where `mcpId` is the full installed plugin
ID. Draft save, publish, duplicate, and import/export retain these values, including disabled
bindings. No selection (including old graphs without `mcps`) means the node is authorized to use no
MCP servers. Malformed bindings are rejected when the executable graph is parsed.

Execution uses the frozen run's enabled IDs throughout Session creation, restore, rebuild, and
refresh. Editing a draft affects later runs only. An unavailable enabled dependency fails the node
with an explicit error; it is never silently skipped. Unselected plugins cannot block the node.
Ordinary chats keep automatic discovery. Package and configuration updates for selected plugins
still use the existing safe refresh boundary; no credentials are persisted in the graph. See
[Session MCP](session-mcp.md) for runtime delivery and refresh behavior.

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

A workflow run freezes one published snapshot and executes directly in a caller-selected Workspace. A project's row targets its Main Workspace; a Task row targets that Task's Isolated Workspace. The run CRUD layer is graph-agnostic. The execution engine owns start/restart/HITL on top of the same repository; `ora-backend` implements `NodeExecutor` as `WorkflowRunNodeExecutor` and composes it at `Backend::open`, where the engine wraps it as the Agent node runtime and registers the swift Start/Condition/Output runtimes behind one registry. After every committed run or node-run state transition, the engine publishes an `AppEvent::WorkflowRunInvalidated { run_id }` on the application event stream; the event carries no workflow state, so frontend run views re-query the persisted run detail and lists instead of rendering from the event. The engine-external interactive transitions — an interactive node parking at awaiting input after its first turn, a human follow-up turn beginning, and that turn ending — commit through a shared transition sink that publishes on the same channel, so every committed node-run transition is observable. Agent output streaming stays on the ACP session stream; invalidation events only signal state changes, and the run view's polling remains as a loss-tolerant fallback.

Before an agent node is prompted, the backend turns the frozen graph and current node-run rows into a structured workflow handoff. The message identifies the current step and its direct neighbors, labels the resolved role as behavioral constraints, lists every node in deterministic topological order with its current status, and keeps the original run request separate. Predecessor outputs are never appended implicitly: the current node receives one only when its custom Prompt explicitly references the corresponding variable, such as `{{#agent-1.output#}}`. Ora's active display locale is frozen when the run is created, so every generated handoff in that run uses consistent Chinese or English copy even if the interface language later changes. User-authored node content and resolved variable values remain verbatim. Enabled skill slash commands remain at the beginning of the first text block because agent CLIs parse them positionally. That block also lists the actual skill-package paths in the selected Workspace and requires the Agent to use those materialized copies. When structured output is enabled, its object-rooted JSON Schema can be configured through either the visual field editor or the raw JSON Schema editor; both edit a dialog-local draft and update the node only when saved. The JSON Schema contract is appended after the ordinary workflow context. It constrains only the final assistant response to one bare JSON object, so the Agent can still reason, call tools, and modify files normally during the step.

Skill delivery is capability-driven. Effect owns physical Skill materialization in every eligible Workspace. A node's enabled Skill bindings are required invocations added to that node's prompt; they are not a security allowlist and do not hide other materialized Skills from the Agent. During run creation, the backend asks an `AgentSkillDeliveryProvider` for each skill-using node's validated, Workspace-relative discovery roots and freezes a per-node receipt containing the original skill id, executable slash-command name, and actual package paths; it does not copy or rewrite packages. Deploy-time skill resolution is origin-aware: a local skill resolves through its formal catalog directory and a plugin-imported skill through the immutable plugin package recorded in its catalog row, so a workflow bound to a plugin skill starts as long as that package is still installed and loadable. The current provider returns the shared `.agents/skills` root for every Agent. A future plugin-backed provider may return different or multiple roots without changing workflow creation, executor, or prompt-rendering code. Node execution consumes only the frozen receipt and never re-resolves skill names from the mutable global catalog; capability or catalog changes therefore affect new runs only.

The session history is the sole source of a node's complete conversation. `workflow_node_runs.output` always stores an Agent node's final assistant text for display, audit, and explicit access through `agent-1.output`, including when structured parsing or schema validation fails and the node is marked failed. The run-scoped variable pool lives in `workflow_runs.payload`: its catalog declares typed Start inputs and stable outputs from data-producing nodes, while enabled structured output additionally declares `agent-1.structured_output`. Condition nodes expose no variables and persist neither `output` nor `selected_branch_id`; their selected branch is private scheduler state in `conditionDecisions`, kept outside the variable pool while remaining restart-safe. A validated structured object is committed only on successful completion; invalid JSON never enters the variable pool. `variablePool.values` contains only assigned values. The run's kickoff instruction remains separate in `workflow_runs.input` and is never exposed as a selectable workflow variable. Start variables may deliberately remain unassigned in the workflow definition and be filled in on the deployed run's pre-start input screen. Each declaration may carry a human-facing display name and string/secret variables may impose a positive maximum character length; the same limit is enforced for editor defaults and deployment-time writes. User-defined workflow globals differ from runtime-owned system globals: each custom declaration requires both an explicit dotted name and a type-correct initial value before it can be saved. For each node, the editor exposes workflow globals plus variables from every direct predecessor. Condition nodes are scope-transparent: their downstream nodes see the original variables from the Condition's direct predecessors, including through a chain of Conditions, without creating a `condition.output`. Other transitive ancestors remain unavailable unless forwarded by a direct predecessor. Structured-output field paths are expanded in the same catalog, so Condition and Output selectors never rely on free-form text. Each terminal Output builds its own result object, so result names must be unique only within that node and may be reused by Outputs on separate branches. The pool is initialized from the frozen graph when a run is created and completed-node writes are committed in the same SQLite transaction as the node status transition. A node Session remains addressable by id for Theater, but standalone Session listing excludes every Session bound to a workflow node run, including node runs soft-deleted by a restart. Workflow ownership survives completion, reruns, and application restarts; retained node conversations never become ordinary chats.

Workflow value types are enforced at graph parsing, editor entry, and variable-pool writes. `array` and `array[any]` both accept heterogeneous JSON arrays; the first is the concise unconstrained declaration and the second explicitly documents an unconstrained element type. Typed arrays validate every element. `file` is a durable Workspace-relative reference shaped as `{ "kind": "workspace_file", "path": "relative/path" }`, and `array[file]` is an array of those references. Editor path strings and legacy saved path strings are normalized into that object, while absolute paths, parent traversal, empty paths, and platform-reserved paths are rejected. Global-value placeholders are generated from this vocabulary, so changing a declaration's type immediately shows a parseable example. Structured Agent schemas use the same types, are validated recursively before execution, and generate a schema-derived valid JSON example in the Agent prompt. Agent-produced file fields must use the canonical object representation shown in that example.

Start inputs keep their form control separate from their variable-pool type. The editor supports text, paragraph, select, number, checkbox, single-file, file-list, and JSON controls, which emit `string`, `string`, `string`, `number`, `boolean`, `file`, `array[file]`, and `object` values respectively. Select choices, required state, and text length limits are frozen into the deployed snapshot and validated again at the execution boundary. Snapshots created before form controls were introduced continue to derive a compatible control from their declared value type.

### Iteration nodes (foreach composite runtime)

The iteration node is the first composite runtime: it owns a region — the set of nodes whose
React Flow `parentId` points at it — and executes that region's frozen subgraph once per element
of an array source. Its `data.iterationConfig` carries `iteratorSelector` (an array-typed
variable), `collectSelector` (a root variable declared inside the region), `errorStrategy`
(`fail` or `continue`), and `maxIterations` (default 50). Region boundaries are validated at
graph parse: the region must be non-empty and entered by an edge from the iteration node, may
contain neither Output nor nested composite nodes, member out-edges must stay inside, no outer
node may target a member, and `maxIterations` must be at least 1.

Rounds are persisted facts. Region rows carry their round in the `iteration` column (the
iteration node's own row keeps `NULL`), so the same node holds one row per round. Each round's
`{iter}.item` and `{iter}.index` bindings commit in the same SQLite transaction as the round's
first node-run rows, and each settled round's ledger entry commits with its continuation (next
round, node completion, or node failure) in one transaction. The engine keeps no iteration
state in memory: the current round is always re-derived from the region rows, so a crash
recovery replays to the same point. The boot sweep is region-aware — interrupted rows inside a
still-running iteration fail as `interrupted_by_restart` while the composite row and the run
survive, and the runtime settles the interrupted round as a failed ledger entry.

Error handling is decoupled into a binary control-flow switch plus a per-round ledger. `fail`
(the default) stops at the first failed round and fails the run; `continue` records failed
rounds in the ledger and still runs the remaining rounds. On completion the node exposes three
variables whose types never depend on the strategy: `{iter}.output` (`array[T]`, where `T` is
the collect target's declared type), `{iter}.entries` (`array[object]`, one
`{item, status, output, error}` envelope per round, aligned with the input array), and
`{iter}.failed_count` (`number`). A source longer than `maxIterations` fails the node at the
startup boundary — never silently truncating — with the length and the ceiling in the error,
and an empty source completes immediately with empty outputs. A round whose branch bypassed the
collect target settles as a failed round (`collect target did not run this round`) instead of
reading the previous round's stale pool value. The node's own failures always propagate to run
failure; only region-internal failures can be absorbed, and Condition decisions inside a region
are recorded per round so a later round can never overwrite an earlier round's branch.

The editor renders the iteration node as an embedded composite region on the same canvas. A
compact header names the iteration while its parameters remain in the Inspector, and a fixed
internal start sits at the left of the region, styled after Dify's iteration-start block: a
44-pixel white rounded card with a blue home badge, plus its entry port on the right edge. The
start cannot be moved, configured, or deleted. Following Dify, the add affordance is a solid
blue circle-plus badge centered on a port: it fades in only while the author hovers the node
(and stays visible while its menu is open or the badge is keyboard-focused), while the start
block body and its port double as the picker trigger — clicking either opens the node picker,
and dragging from the port still starts a connection, so the decorative badge never intercepts a
drag.
Authors can add an Agent or Condition this way from the internal start, or append to an
unconnected member output (including a specific Condition branch), and create multiple entry
branches by dragging from the start's source handle. They can also insert on
an internal edge other than the entry edge. The entry edge does not repeat a midpoint plus button because the
fixed start already owns that insertion seam. The internal canvas has one container boundary,
without a second dashed region frame. Selecting the frame repaints only its border and shadow —
the background fill never changes between selected and unselected states. The internal start is presentation-only: the frozen graph
still records an entry as
`iteration --iteration-entry--> member` and never gains another runtime node. Each insertion
creates or rewires its edges in the same undo/autosave operation. Dragging never changes
`parentId`: members stay inside their owner, and an outer node dropped over a region returns to
its original position with guidance to use the internal add entry. React Flow parent constraints
are derived while rendering and are not persisted.

Expanded frames keep a 560×340 minimum and persist fitted `initialWidth` / `initialHeight` values.
Authors can also resize an expanded frame manually, Dify-style: the bottom-right corner
shows a soft gray arc affordance (revealed on frame hover or while the frame is selected), the
cursor turns into a resize indicator over the corner, and dragging with the left button resizes
the frame in 20-pixel grid steps; the gesture is one undoable, auto-saved edit that never shrinks
below the minimum or clips region members. Collapse hides the internal start, members, and internal edges only in the canvas projection, so
the source graph remains unchanged and expansion restores the same size. Frames grow when members
are inserted or moved and compact after deletion or automatic layout. Because a freshly inserted
card is only measured after it renders, the insertion-time fit re-runs when real measurements
arrive, so React Flow's parent extent never clamps a tall member back over the region's internal
affordances. New members stack below the measured bottom of existing members and are nudged below
any card they would overlap, because card heights vary by kind and content. Automatic layout arranges every region DAG
independently before laying out the outer graph with the fitted container dimensions. Deleting a
non-empty frame confirms its member count, then removes the frame, members, and incident edges as
one undoable edit. Deleting the current `collectSelector` target clears that selector and asks the
author to configure a replacement. Region Agent nodes default to `interactive: false`; legacy
interactive members remain visible with a direct repair action because the runtime still rejects
them.

The variable catalog follows region scope — members see `item`/`index` plus their in-region
upstream products but never the node's own exposed results, while outer consumers see the three
exposed variables but never the round bindings. Run views group region states by
`(node_id, iteration)`. Theater keeps only the iteration container on its top-level path and
projects the frozen region DAG beneath it: multiple `iteration-entry` targets stay in a persistent
parallel group, Condition successors are labeled as conditional branches, and later dependencies
form following stages. One region-level round selector drives every member's status, conversation,
and detail view; switching parallel peers preserves the round, while a member that did not run in
that round is shown explicitly instead of borrowing another round's result. The selected member's
region, round, and parallel position remain visible above its act card. Records without round
projections still receive the structural grouping and use their node-level status. The overview
reconstructs the iteration parent frame from the frozen `parentId`, `initialWidth`,
`initialHeight`, and `iteration-entry` edge facts, so completed member cards remain inside their
region and entry edges remain visible. It supports mouse-wheel, pinch, plus/minus, and fit-to-graph
zoom controls.
The production regression suite exercises the same boundaries through SQLite and the fake ACP
provider: a second round receives a new session, round bindings are available while rendering its
prompt, and synchronous failures settle under both `fail` and `continue` without leaving a run
stuck.

### Failure visibility and resuming from failure

Failed nodes stay visible. When a node fails, the run fails immediately (D2) while any
in-flight siblings keep running to completion and remain bindable; the scheduler then
dispatches nothing on a `Failed` or `Cancelled` run. `payload.error_detail` on the failed
node-run records `kind`, `message`, `source_chain`, `attempt`, `resumable`,
`injects_previous_failure`, `auto_retryable` (whether automatic retry covers this kind, see
below; absent on rows written before the field existed), and `recorded_at`. `kind` is a mechanical classification, never
inferred by a model.

`resumable` predicts whether re-running the same snapshot is a sensible first move
(environment / transient), not whether the UI allows resume — resume is always offered for a
failed or cancelled idle run:

| Kind                                                                                                                                                                                                    | `resumable` |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `workflow_model_not_found`, `missing_agent_config`, `session`, `session_ended_without_stop_reason`, `session_binding_rejected`, `interrupted_by_restart`, `repository`, `baseline_persist`              | true        |
| `structured_output`, `agent_refusal`, `prompt_template`, `missing_agent_ref`, `missing_skill_materialization`, `invalid_run_payload`, `unknown_stop_reason`, `multiple_outputs`, `condition_evaluation` | false       |

Only agent-behaviour failures are injected into a later prompt (`injects_previous_failure`):
`structured_output`, `agent_refusal`, `unknown_stop_reason`, `multiple_outputs`. A run created
with `injectLastFailure: false` injects nothing, so its failures record
`injects_previous_failure: false` for every kind and the run view does not promise injection.

Resume soft-deletes the failed or cancelled node runs and all of their descendants
(`is_deleted = 1`) and reschedules from the surviving state. Attempt numbering counts those
soft-deleted predecessors per `(run_id, node_id, iteration)` (`iteration IS NULL` for outer
rows); a Loop body row counts only the rows of its own Loop round, so every round, including the
rounds a Loop resume reruns, numbers its attempts from 1. `find_last_failed_attempt` looks up
`(run_id, node_id, iteration)` without the round restriction, so the first attempt after a Loop
resume is still told about the failure that failed the Loop.

Each node records a pre-node git checkpoint under `refs/ora/checkpoints/<node_run_id>` before
it runs. Rollback first snapshots the worktree as `pre-rollback-<run>-<ts>` so the operator
can undo. Provenance lives on the node-run payload as `checkpoint`, `checkpoint_error`, and
`file_changes`. The three rollback modes are `keep` (leave the worktree), `node_files`
(restore only paths the failed nodes recorded), and `checkpoint` (restore the whole worktree
to the resume unit's checkpoint). `node_files` is unavailable with
`nodeFilesUnavailableReason` `"no_file_changes"` when a failed node has no checkpoint or
recorded changes, and `"composite_region"` when the resume unit is a composite (Iteration or
Loop).
`checkpoint` is unavailable with `"no_checkpoint"`, `"siblings_ran_after_checkpoint"` (a live
node outside the resume unit was still active after the unit's earliest start: `finished_at`
is none or later, or `started_at` is later; Start/Condition/Output rows that finished before
that instant do not count), or `"not_resumable"`.

A node that automatic retries replaced is rolled back as one unit with every attempt of its
retry chain (`payload.retry_chain` on the waiting row: the earlier attempts of the same node and
round since the last start, restart, or resume, oldest first). `checkpoint` restores the
worktree from before the chain's first attempt, and `node_files` covers the files any attempt
changed and is offered only when every attempt that ran took a checkpoint. A chain of one
attempt behaves as before.

The run-level switch `inject_last_failure` (default on) injects the previous attempt of the
same `(node_id, iteration)` into the prompt when that attempt's kind is one of the four
injectable kinds. The rendered block is stored as `payload.injected_failure_context`.

A failed or cancelled run may resume on a newer published snapshot when the two graphs are
compatible: removed nodes, changed node types, and Start-contract changes are incompatible
(`node_missing:<id>`, `node_type_changed:<id>`, `start_node_changed`,
`start_variables_changed`, `variable_type_changed:<selector>`, `variable_missing:<selector>`).
A succeeded iteration composite that is not in the resume unit cannot change its
`iterationConfig` or region member set (`iteration node <id> changed after it completed`); a
composite that is itself the resume unit may change freely because the loop restarts. The
snapshot the node last ran is recorded as `payload.snapshot_id`.

On-demand AI diagnosis writes `payload.ai_diagnosis`. It is labelled as a guess and is never
read by scheduling, resume, rollback, or snapshot-switch decisions.

The resume unit for anything inside a region is the owning composite node. When any
failed or cancelled row belongs to a region (`iteration IS NOT NULL`), or the composite's own
row is failed or cancelled, resume soft-deletes the composite row, every region row of every
round, every ledger entry and every pool binding the composite wrote (`{iter}.item`,
`{iter}.index`, and the exposed `{iter}.output` / `{iter}.entries` / `{iter}.failed_count` if
present), and all outer descendants of the composite, then reschedules; the loop restarts from
round 1. Partial in-loop resume is out of scope. `node_files` rollback is unavailable for that
unit (`composite_region`); `checkpoint` restores the worktree to the composite's pre-loop
checkpoint. The boot sweep stays region-aware for interrupted region rows
(`interrupted_by_restart` on the member, composite and run survive, the round settles as
failed); whole-run `InterruptedByRestart` handling remains for non-region rows.

Loop containers (`kind: "loop"`, see [Loop containers](#loop-containers)) resume the same way:
the Loop node is the resume unit for any failed or cancelled Loop body row (including a retry
wait the run abandoned) and for its own failed or cancelled row, and clearing it also closes its
round scopes and soft-deletes every node run those rounds created, so the rerun starts from
round 1 with no active round. A Loop is never resumed inside an open round. `node_files`
rollback is unavailable for that unit (`composite_region`).
Inside a round the Loop keeps its own failure semantics (siblings in the round are cancelled and
the failure climbs to the Loop node); D2 sibling survival applies to the root scope.

### Automatic retry of failed agent attempts

An agent node may retry a failed attempt by itself before the failure reaches the run. The policy
is `agentConfig.retry = {enabled, maxRetries, initialDelaySeconds}` (camelCase in the graph):
`maxRetries` is an integer from 0 to 5 and `initialDelaySeconds` an integer from 0 to 300. A
missing (or `null`) `retry` means `{enabled: true, maxRetries: 2, initialDelaySeconds: 10}`; a
present object must carry all three fields, and an incomplete or out-of-range one is rejected
when the graph is parsed (`node <id> has an invalid retry config: …`). The field is optional
with a default, so `schemaVersion` is unchanged and older graphs parse as before.
`enabled: false` or `maxRetries: 0` turns retry off.

Only agent nodes retry, including agents inside Iteration regions and Loop bodies (each retries
on its own); Start, Condition, Output, and composite nodes never do, nor does an agent with
`interactive: true`. Only these kinds retry: `session`, `session_ended_without_stop_reason`,
`session_binding_rejected`, `structured_output`, `agent_refusal`, `unknown_stop_reason`. The
other kinds (`missing_agent_ref`, `workflow_model_not_found`, `missing_agent_config`,
`invalid_run_payload`, `prompt_template`, `missing_skill_materialization`, `baseline_persist`,
`repository`, `interrupted_by_restart`, `multiple_outputs`, `condition_evaluation`) fail at once.

Retry `n` (1-based) waits `initialDelaySeconds × 2^(n-1)` seconds, capped at 600: with the
default policy the waits are 10 s and 20 s, so a node runs at most three attempts.

When a covered failure arrives, one transaction records the failed attempt's full
`payload.error_detail`, soft-deletes it exactly as a resume clears an attempt, and inserts the
next attempt in the same run, scope, and iteration as a _waiting_ row: status `Running`,
`started_at` null, and `payload.retry_wait = {attempt, max_attempt, retry, max_retries,
delay_ms, scheduled_at, due_at, previous_node_run_id}`. Because the row is `Running`, the run
stays `Running`, independent branches keep going, successors keep waiting, and the run is not
drained. When `due_at` passes, the backend timer (one tokio sleep per wait, no state of its own)
wakes the engine under the run lock: the marker is removed, `started_at` is set, and the attempt
is dispatched against the same scope it failed in (the outer graph and run pool, the iteration
round's bindings, or the Loop body and that round's pool). Every wake re-reads the persisted
marker, so a wake that arrives early re-arms, and a wake after cancel, run failure, a restart,
or an earlier wake is a no-op; several waits simply own separate deadlines.

The retried attempt gets the previous-failure prompt block under the run-level
`inject_last_failure` switch when the failed kind is injectable (see above); session-class
failures inject nothing. There is no rollback before a retry; the new attempt records its own
pre-node checkpoint. `find_last_failed_attempt` ignores a failure that a later succeeded attempt
of the same `(node_id, iteration)` already recovered from, even after a resume or restart
cleared that attempt, so the next Loop round (or a Loop restarted from round 1) does not inherit
a failure that was already retried away. It also ignores rows that never started (a waiting row
that a restart failed), so the attempt that really failed is the one injected. The prompt stall resend inside one session is a
separate mechanism and is unchanged.

The retries a row has used ride on `payload.auto_retry = {retry, max_retries}`. Rows without it
— a first attempt, a manually resumed attempt, an attempt after a restart — start a fresh
budget, while `error_detail.attempt` keeps counting across all of them (a resume after three
failed attempts runs attempts 4, 5, 6). Inside a Loop both the budget and the numbering start
over in every round.

A wait ends early in these cases:

- **Cancel** settles every waiting row as `Cancelled` at once; nothing starts afterwards.
- **Run failure** (a root-scope node's final failure, a Loop's failure, an iteration under
  `fail`) settles every waiting row of the run as `Cancelled` with error
  `{"reason":"retry_abandoned"}`, so the retry never fires and resume reruns the node. A
  composite row that D2 leaves running keeps its round open; resume then restarts that Loop or
  Iteration from its first round (the composite resume rules above).
- **Restart**: the boot sweep treats a waiting row as a running one. An outer row fails with
  `interrupted_by_restart` and fails the run; a row inside a running iteration is absorbed and
  its round settles as failed. The retry does not continue.

Inside an Iteration or Loop the retry stays in the same round; the composite's error strategy
only sees the exhausted failure. A sibling region row's final failure does not abandon a waiting
retry in the same round: the round settles once it is drained, as for any in-flight row.

`get_workflow_run` exposes both states. A node is waiting when its live row is `running` and
`payload.retry_wait` is present; the countdown is `due_at - now` and the label is
`attempt / max_attempt`. `GetWorkflowRunResponse.failedAttempts` lists the earlier failed
attempts (soft-deleted `Failed` rows with their `error_detail`) oldest first, each with
`nodeRunId`, `nodeId`, `scopeId`, `iteration`, `sessionId`, `attempt`, `kind`, `message`,
`sourceChain` (the `error_detail.source_chain`, outermost first; for session failures the
agent's own reason is there, because `message` is generic), `recordedAt`, `startedAt`, and
`finishedAt`. Attempt numbers count every soft-deleted row of the node and iteration in the run (for
a Loop body row, in its Loop round), whatever its status, while `failedAttempts` lists only the soft-deleted `Failed` rows that carry
an `error_detail`. Both continue across resume and restart (a Loop body row's number restarts with each
Loop round, including the rounds a Loop resume reruns), and the retry budget restarts after
either (see `auto_retry` above). Listed numbers can therefore skip: a cancelled or abandoned
attempt, a succeeded attempt that a composite resume cleared, or a failed row without a failure
record takes a number but is not listed.

#### Retry in the app

**Settings.** In the workflow editor, the agent node's settings end with a "Retry on failure"
section after Structured output: a switch, "Max retries" (0–5) and "First wait (seconds)"
(0–300). The two inputs keep their values but are disabled while the switch is off. A short list
under the header names the retried failures, says that a retry after a reply problem tells the
agent why the previous attempt failed, that each wait doubles up to 600 seconds, that file
changes are not rolled back, and that interactive nodes never retry. While `retry` is absent the
section shows the defaults and writes nothing; the first change writes the complete
`{enabled, maxRetries, initialDelaySeconds}` object. An invalid number stays in its input with
a message and is not written. The section is hidden while the node is interactive (a stored
`retry` is kept). In a run, the node inspector shows the effective policy as a read-only
"Retry on failure" row: "Default: up to 2 retries, first wait 10 s", "Up to 4 retries, first
wait 30 s", "Off", "Not retried (max retries is 0)", or "Not retried (interactive node)".

**Waiting state.** The run view shows a waiting row (live row `running`, `started_at` null,
`payload.retry_wait` present) as its own status, "Waiting to retry", in orange, instead of
running. It has no spinner, no pulsing frame, and no start time or duration, because nothing
is executing and the row has no session yet. The stage card, the Overview node, and the node
inspector header show "Waiting to retry (attempt {attempt}/{max_attempt}), starts in {n}s",
where `n` is recomputed from `due_at - now` every second; at zero the label reads "…, starting…"
until the next run refresh (the run detail is polled every 1.5 s) shows the started attempt.
Path chips, iteration member chips, parallel chips, and Loop round rows use the same orange
status and show only "starts in {n}s"; the chips' accessible names and a screen-reader-only text
in the Loop round row add "Waiting to retry (attempt {attempt}/{max_attempt})" without the
seconds. The session panel of a waiting node explains that the new attempt's session appears
once it starts. When no node is pinned, the stage can follow a waiting node like a running one,
but an awaiting-input or running peer takes precedence: among active nodes the stage prefers the
most recently started one, and a waiting node has no start time. Progress counts ("done /
total", "x/y complete" for an iteration round) do not count a waiting node as done.

**Attempt history.** Below the current attempt's error block, the node inspector lists
"Earlier failed attempts" from `failedAttempts`, oldest first, for the node execution being
viewed (the node and round of an outer or iteration row, including attempts from before a
"Run again from start", or the Loop round of a Loop body row). Each entry shows "Attempt {n}",
the translated failure kind, the message, the last `sourceChain` entry as "Underlying error"
with the whole chain in an expandable list when it has more than one entry, the attempt's
start and end time (or the time the failure was recorded when it never started), and its round
for iteration and Loop rows. An entry is tagged only when the run detail proves what replaced
it: the live row's `auto_retry.retry` counts the automatic retries directly before it, and a
fresh row whose attempt number directly follows a failed attempt replaced it by resume
("Resumed manually"). An attempt replaced by an automatic retry reads "Retried automatically"
when the row that retry created started, and "Automatic retry scheduled (not started)" when
that row never started (it is still waiting, or its wait was cancelled, abandoned, or ended by
an app restart). A gap in the numbers stops the tagging, so nothing older is tagged; the missing
number belongs to a row the list does not include (a cancelled or abandoned attempt, a
succeeded attempt that a composite resume cleared, or a failed row without a failure record).
Attempts from before a "Run again from start" are tagged "Before “Run again from start”"
instead. The current attempt keeps its own display, including the injected
previous-failure block. Failed attempts of Loop rounds that a Loop resume closed are not shown,
because no live row of those rounds remains.

**Exhausted retries and retries that never started.** A failed node whose live row carries
`auto_retry.retry = n` shows "Retried automatically {k} time(s), still failed" next to the
resume hint, where `k` counts only the retries that started: `n` when the row started and
`n - 1` when it never did (a waiting row that an app restart failed); nothing is shown when `k`
is 0. A failed or cancelled row that carries `auto_retry` but never started shows "This
automatic retry was scheduled but never started". A row cancelled with
`{"reason":"retry_abandoned"}` shows "The run ended while this node was waiting to retry, so the
retry never started" instead of the raw error and no second note.

**Failures that are never retried.** When an agent whose retry policy is on (enabled, with at
least one retry, not interactive) fails with a kind automatic retry does not cover
(`error_detail.auto_retryable` is false), the failure block adds "This kind of failure is not
retried automatically", so an immediate failure does not look like a broken policy. Rows
without `auto_retryable` show nothing extra.

### Entities and tables

| Domain type              | Backing table               |
| ------------------------ | --------------------------- |
| `WorkflowRun`            | `workflow_runs`             |
| `WorkflowNodeRun`        | `workflow_node_runs`        |
| `WorkflowExecutionScope` | `workflow_execution_scopes` |

`WorkflowRun` pins `snapshot_id` to the user-released version it was created against and stores its own display name and `workspace_id`. `WorkflowNodeRun` records one executed node and its scope; nodes that never started have no row, and the frontend derives "not started" by comparing graph nodes against recorded node runs. `WorkflowExecutionScope` records a Loop parent, round index, lifecycle, and private round state. Run and node status share the same five-value enums (`Pending | Running | Succeeded | Failed | Cancelled`). An interactive node parked awaiting follow-up input is persisted as `Pending`; the public contract derives a `Running` run with an awaiting node as `AwaitingInput` so the sidebar can surface that human action is needed. A session bound to a terminal node is read-only: the backend rejects new prompts against it.

### Creation and snapshot pinning

`CreateWorkflowRunHandler` admits the requested active `workspace_id`, resolves the frozen snapshot — an explicit `snapshot_id` or the workflow's `published_snapshot_id` — validates it belongs to the requested workflow together with the declared roles and Skill bindings, and persists the run. Runs start `Pending` with an empty `current_nodes` anchor. Creation compiles the run-scoped variable pool from the frozen graph and seeds the reserved `{start_id}.input` selector with the resolved kickoff text, so a node prompt can render the run instruction as a template variable as well as receiving it in the assembled workflow context. When the caller supplies no kickoff input, the run inherits the frozen snapshot's Start node instruction as its default input. Deployment gathers the run name, an editable run instruction, and typed values for any declared Start input variables before the run starts. The frontend obtains the exact Workspace from the project or Task row that opened the workflow picker; the backend does not infer a branch or create another worktree.

### Reading and deleting

Get returns the run with its display name and node runs; list returns project-scoped summaries. Node-run history is read-only in this layer — the engine owns node-run writes and the state machine.

Deletion refuses active runs — a `Running` run, a HITL pause with a non-terminal node run, or a `Running` session bound to one of the run's nodes — and then soft-deletes the run, its node runs, and node-owned sessions. A not-started `Pending` run (empty `current_nodes`, no node rows) can be discarded without cancelling. The selected Workspace is shared infrastructure and is never deleted with one run. Soft-deleted runs are invisible to queries and cannot be reactivated.

### Snapshot protection

A published snapshot referenced by a live run cannot be soft-deleted (`SnapshotInUse`), and deleting a workflow whose snapshots a live run freezes is refused (`ActiveRuns`), so a run's frozen graph stays readable across its lifecycle.

## Boundaries (non-goals)

- Workflow-run CRUD persists runs and node-run records but does not execute them. Graph execution, node-run writes, and the state machine (start/restart/HITL) are engine-owned in `ora-application`, with agent-node sessions driven by `ora-backend`'s `WorkflowRunNodeExecutor`. The initial runtime variable pool is deliberately stored in the existing `workflow_runs.payload` JSON column; no database migration or separate variable table is required.
- Graph validation at run start belongs to the workflow run engine, not the definition CRUD handlers.
- Tauri command registration and web-server route wiring are transport concerns owned by the respective adapters.

See [Domain Models](domain-models.md), [Application and Contracts Boundary](application-contracts-boundary.md), [Database Repositories](database-repositories.md), [Workflow Run Engine](../crates/application/src/workflow_run/engine/README.md), [ora-backend](../crates/backend/README.md).

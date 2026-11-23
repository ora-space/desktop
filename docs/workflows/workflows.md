# Workflows

English | [中文](workflows.zh.md)

A workflow is an editable agent orchestration graph with an editable draft and immutable published
snapshots. Definitions are owned by `crates/application/src/workflow`, persisted by `ora-db` and
edited in `packages/app-shell/src/features/workflow-editor`. Executing a snapshot is described in
[Workflow runs](workflow-runs.md); Loop and Iteration containers in
[Loops and iterations](loops-and-iterations.md).

## Definitions and versions

`Workflow` (`workflows`) holds the stable identity: name, published snapshot pointer and audit
fields. `WorkflowSnapshot` (`workflow_snapshots`) owns the versioned React Flow graph. Read models
keep graph data out of lists, and the library lists newest first.

- Every workflow has exactly one draft snapshot, created with the workflow. The draft is a
  workspace: `updateDraft` changes its graph in place without a new row.
- **Publish** copies the draft into a new immutable snapshot and always activates it
  (`workflows.published_snapshot_id`), making it the version future runs use. A published
  snapshot's `updated_at` stays `NULL`.
- **Rollback** copies a historical snapshot's graph into the draft without changing the published
  pointer.
- **Activate** switches the published pointer to another snapshot and syncs its graph into the
  draft.
- **Delete snapshot** removes a published snapshot but refuses the draft, the active version and any
  snapshot a live run references (`SnapshotInUse`). Deleting a workflow whose snapshots a live run
  freezes is refused (`ActiveRuns`); otherwise deletion soft-deletes the workflow and its snapshots.
- **Duplicate** creates a new identity and draft from the source's current draft, without published
  snapshots, the active pointer or runs.

Versions are strings. The draft is the reserved `"draft"`. Published versions are user-provided
(non-blank, at most 128 bytes, a single URL path segment, not `.` or `..`) or derived as
`v{timestamp_millis}` from the application clock, with a numeric suffix on a same-millisecond
collision. A partial unique index prevents duplicate visible versions per workflow, so a deleted
version name can be reused.

Operations: `createWorkflow`, `getWorkflow`, `listWorkflows`, `updateWorkflow` (rename),
`deleteWorkflow`, `getDraft`, `updateDraft`, `publishWorkflow`, `rollbackWorkflow`,
`activateWorkflow`, `listVersions`, `getVersion`, `getWorkflowSnapshot`, `deleteSnapshot` and
`analyzeWorkflow`. Files are exchanged through the native `write_workflow_export` and
`read_workflow_import` commands; packaged workflows arrive as [workflow packages](../plugins/workflow-packages.md).

## Graph storage

The `graph` column stores the complete React Flow document. Saving a draft stores it as an opaque
string, so an unfinished canvas can be kept. Publishing runs the same executable-graph parse as run
creation over the entry-reachable subgraph (see [the run engine](../../crates/application/src/workflow_run/engine/README.md)):
a reachable node whose configuration the parser rejects fails publish with
`workflow_run_graph_parse` and creates no snapshot, while spare nodes off that path may stay
incomplete. Run creation parses the frozen snapshot again.

### Loading in the editor

Every load path (draft hydration, version preview, export and the run view) parses through one
shared boundary in `@ora/workflow-runtime`, `parseWorkflowGraphWithReport`:

- Nodes that are not usable records, or whose `data.kind` is outside `WORKFLOW_NODE_KINDS`, are
  dropped together with the edges that referenced them. The editor reports the count and kinds
  ("Skipped 1 nodes this version cannot render (router)"); read-only views drop silently.
- Legacy `prompt` and `model` nodes are upgraded to `agent` before the kind check.
- Dropping happens on read and never rewrites stored bytes, so a published snapshot keeps the
  original and rollback or re-import restores skipped nodes. The draft does change, because the next
  autosave writes the normalized graph.
- Presentation geometry is restored in memory: invalid coordinates get deterministic positions in
  their scope, container bounds fit their children, unusable edges are omitted, missing or duplicate
  edge ids are replaced, invalid viewports fall back to the origin, and cyclic or dangling visual
  parents are detached without changing `data.containerId`. Run analysis still uses the original
  frozen graph, so recovery never hides structural errors.
- Agent configuration is projected safely: missing or mistyped values display as empty placeholders
  and no executor or model is invented. Frozen snapshots are never rewritten.
- An unexpected render failure is isolated to the workflow pane, which offers a return action while
  the sidebar stays usable.

`deno run -A scripts/repair_workflow_snapshots.mjs --database <path>` previews layout repairs of
active drafts that no run or published version references; `--apply` applies them after a complete
SQLite backup. Published and run-referenced snapshots are never modified, and ambiguous ownership is
reported instead of changed.

## Node kinds

| Kind | Role |
|---|---|
| `start` | The entry; declares typed Start inputs and the default run instruction |
| `agent` | Prompts an agent session, see [Workflow runs](workflow-runs.md#agent-nodes) |
| `condition` | IF / ELSE IF / ELSE branching on typed comparisons |
| `aggregator` | Passes the first assigned of several branch outputs through as one variable |
| `output` | Builds the run's result object |
| `loop`, `loopExit` | Feedback containers and their explicit exit, see [Loops and iterations](loops-and-iterations.md) |
| `iteration` | Foreach container over an array |

The editor catalog also contains `tool`, `junction`, `human` and `subflow`. The run engine does not
execute them: a reachable one fails publish or run start.

Executable Loop graphs use `schemaVersion: 2`; flat snapshots keep their original decoder behavior.

### Variable aggregator

`data.aggregatorConfig.variables` is an ordered list of root selectors `["nodeId", "root"]`. The
first **assigned** candidate in declaration order passes through unchanged as `{agg}.output`;
assigned means the key exists in the pool, so `null`, `false`, `0`, `""`, `[]` and `{}` all count.
If none is assigned the run fails with `aggregator_no_match`. Every candidate must be produced by a
static transitive predecessor (sibling branches qualify) or be a global, and all candidates share
one declared type. The node never reads Condition decisions; branch selection is expressed by the
existing branch projection plus pool facts. Groups, nested paths and type coercion are not supported.

## Nodes excluded from execution

Authors can keep spare nodes and connected groups beside the execution path. Drafts, snapshots,
rollback and import/export keep the complete canvas; execution derives an entry-reachable subgraph
from the frozen snapshot without rewriting it.

- Root reachability starts at Start; active Loops use their child Start and active Iterations their
  entry edges. Every member of an unused container is excluded. Static analysis follows every
  Condition outlet; runtime branch selection is separate.
- Spare nodes never enter scheduling, variable pools or role and Skill preparation, and create no
  node run or session. An edge from a spare node into an active node is excluded so it cannot block
  a join, and an active node that references a spare output fails validation. Reconnecting a node
  restores normal validation.
- Document-wide checks still reject duplicate ids, dangling edges, invalid ownership and illegal
  cross-scope edges.
- `analyzeWorkflow` drives the editor's "Excluded from execution" badges and count and the run
  overview. Unrecognized kinds are reported separately because loading already dropped them.
  Membership is derived from topology, never persisted, and analysis results are bound to document
  identity so switching documents cancels stale requests.

## Agent node configuration

The Agent inspector edits the node's executor and model, role, prompt, Skill and MCP bindings,
structured output, interactivity, retry and prompt inactivity. The runtime meaning of each is in
[Workflow runs](workflow-runs.md#agent-nodes); the stored shapes are:

- **MCP bindings**: `mcps: [{ mcpId, enabled }]`, where `mcpId` is the installed plugin id. Only
  enabled ids form the node Session's allowlist; no selection (including graphs without `mcps`) means
  no MCP servers. The inspector lists installed `kind: "mcp"` plugins; missing plugins stay visible by
  id and can be disabled or removed. Malformed bindings fail executable parsing.
- **Absent config**: a graph is JSON, so an Agent node can carry `"agentConfig": null`; the
  backend imports such a document as-is. `null` means the node declares no executor, role or
  bindings, exactly like an absent field: the editor opens it with the same default a new Agent node
  gets, and the run inspector shows no agent section. Opening the graph never rewrites the `null`
  into a materialized config.
- **Retry**: `agentConfig.retry = { enabled, maxRetries, initialDelaySeconds }`, with `maxRetries`
  0–5 and `initialDelaySeconds` 0–300. Absent or `null` means
  `{ enabled: true, maxRetries: 2, initialDelaySeconds: 10 }`; a present object must be complete.
  The section is hidden for interactive nodes.
- **Prompt inactivity**: `agentConfig.promptInactivity: "timeout" | "wait" | "unbounded" | { waitMinutes: number }`; `"unbounded"` explicitly disables the silence limit. Absent or `null` means
  `"timeout"`. Opening an old graph never adds the field.
- **Structured output**: an object-rooted JSON Schema edited visually or as raw JSON in a dialog that
  updates the node only on save.

Save, publish, duplicate and import/export keep these values verbatim, including disabled bindings.
None of them needs a schema-version or database migration, and editing a draft affects later runs
only.

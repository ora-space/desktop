# Loops and Iterations

English | [中文](loops-and-iterations.zh.md)

Workflows have two container kinds. A **Loop** repeats a body with typed feedback until a condition,
an explicit exit or a round limit ends it (generate → review → revise → review). An **Iteration**
runs its region once per element of an array (foreach). Both keep the outer graph and every single
round a DAG; nested containers are not supported. General workflow and run behavior is in
[Workflows](workflows.md) and [Workflow runs](workflow-runs.md).

## Iteration

An `iteration` node owns a region: the nodes whose React Flow `parentId` points at it. Its
`data.iterationConfig` carries:

| Field              | Meaning                                                                                                                |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `iteratorSelector` | An array-typed variable to iterate                                                                                     |
| `collectSelector`  | A root variable declared inside the region, collected each round                                                       |
| `errorStrategy`    | `fail` (default) stops at the first failed round and fails the run; `continue` records failed rounds and runs the rest |
| `maxIterations`    | Safety ceiling, default 50, at least 1                                                                                 |

Region boundaries are validated at parse: the region is non-empty and entered by an edge from the
iteration node, contains no Output and no nested container, member out-edges stay inside, and no
outer node targets a member.

**Rounds are persisted facts.** Region rows carry their round in `workflow_node_runs.iteration`
(migration `0012`; the iteration node's own row keeps `NULL`). A round's `{iter}.item` and
`{iter}.index` commit in the same transaction as its first rows, and each settled round's ledger
entry commits with its continuation (next round, completion or failure). The engine keeps no
iteration state in memory: the current round is re-derived from the rows, so recovery replays to the
same point.

**Results.** On completion the node exposes `{iter}.output` (`array[T]`, `T` being the collect
target's type), `{iter}.entries` (`array[object]`, one `{item, status, output, error}` per round
aligned with the input) and `{iter}.failed_count` (`number`); their types never depend on the
strategy. A source longer than `maxIterations` fails the node at startup with the length and ceiling,
never truncating; an empty source completes immediately. A round whose branch skipped the collect
target settles as failed instead of reading a stale value. The node's own failures always fail the
run; only region failures can be absorbed. Condition decisions are recorded per round.

**Scope.** Members see `item`, `index` and their in-region upstream products but not the node's own
results; outer consumers see the three results but not the round bindings.

**Editor.** The node renders as an embedded region with a compact header and a fixed internal start
(a small card with an entry port) that cannot be moved, configured or deleted; it is presentation
only, and the frozen graph records entries as `iteration --iteration-entry--> member` edges. Authors
add members from the start or from a member's output port, create several entry branches, and insert
on internal edges; each insertion is one undoable, autosaved edit. Dragging never changes `parentId`.
Frames keep a 560×340 minimum, persist fitted `initialWidth`/`initialHeight`, resize from the
bottom-right corner in 20-pixel steps without clipping members, collapse without changing the graph,
and grow or compact as members change. Deleting a non-empty frame confirms the member count.

**Run views.** Run views group region states by `(node_id, iteration)`. Theater keeps only the
container on its top-level path and projects the region DAG beneath it; one round selector drives
every member's status, conversation and details, and a member that did not run in a round says so
instead of borrowing another round. The overview reconstructs the frame from the frozen geometry and
supports wheel, pinch, button and fit-to-graph zoom.

## Loop

A root `loop` node owns `data.loopConfig`; every child declares the Loop through both React Flow
`parentId` and `data.containerId` (schema version 2). Each body is a separate DAG with exactly one
reachable child Start.

| Field                   | Meaning                                                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `maxIterations`         | Round limit, 1–100                                                                                                                                      |
| `maxIterationsBehavior` | `fail` (default) or `succeed` when the limit is reached without another ending                                                                          |
| `variables`             | Carried variables: `name`, `valueType`, `initial` (`{ "kind": "constant", "value" }` or `{ "kind": "variable", "selector" }`) and a `feedback` selector |
| `until`                 | End conditions in the Condition `logic`/`conditions` syntax; may be empty                                                                               |
| `outputs`               | Named exports, `{ "name": "result", "variableSelector": ["child", "output"] }`                                                                          |

Initializers can read globals and upstream outer nodes. Feedback, conditions and exports read the
completed body, carried values, globals and upstream outer values. Outer nodes read only the Loop's
exports, never its children. Nested Loops, ownership mismatches, cross-scope edges and selectors, and
invalid types on active nodes are rejected before any session starts.

**Rounds.** Each round gets a fresh variable pool that imports only globals and upstream outer
values, and a new node run and session per node. After the body drains, the engine resolves every
feedback assignment from the same completed pool, evaluates `until` against that round, and in one
repository transaction either commits the next round's scope or completes the Loop with its exports.
A missing value fails explicitly; a previous round's output is never substituted. Cancellation and
child failure settle the active round and the Loop together, and inside a round a failure cancels the
round's siblings and climbs to the Loop node. The Workspace is shared, so file edits accumulate across
rounds.

**End conditions.** Conditions are checked after each round: a match succeeds the Loop, otherwise the
next round starts. The panel offers operators filtered by variable type, a boolean picker, unary
operators without a value, typed constants or `{{#node.variable#}}` templates (a value that is only
one placeholder keeps the bound JSON type), and AND/OR across rows. Emptiness and existence checks
accept unset variables; other comparisons fail rather than reuse an earlier round's value. A failure
at the limit keeps the exact comparison and appends a short reading of the last round's inputs (at
most 160 characters each). Conditions may be removed entirely, leaving the exit node and the limit.

**Round limit.** With **Stop workflow at maximum rounds** on (`fail`), exhausting the rounds fails the
Loop and the workflow. With it off (`succeed`), the last completed round publishes the exports,
succeeds the Loop with `stop_reason: loop_max_iterations` and releases downstream nodes. Older
snapshots without the field use `fail`; runs use their frozen value, including after restart.

### Exit node

`loopExit` ("Exit loop") belongs to one Loop, accepts incoming edges and has no output port; put a
Condition before it to decide when to exit. Several branches may have their own exits. Root workflows
and Iteration regions cannot contain one.

Reaching an exit stops the owning Loop, not the workflow. The scheduler dispatches ready exits before
their siblings in the same wave. One `RequestExit` transaction records the exit, its time and the
frozen exports from the current committed round, completes the exit node and cancels active siblings;
completed nodes keep their output and history and unstarted branches are not dispatched. The exit
reads no next-round feedback and evaluates no end condition. The first reachable exit in the graph's
ready order wins.

The session executor then stops only the cancelled sessions of that round and waits for their drivers,
outside the run gate. A guarded acknowledgement publishes the exports and completes the Loop with
`stop_reason: loop_exit` before the outer graph continues. Cleanup failure or a 30-second timeout
fails the run instead of reporting a successful exit; external side effects are not rolled back. The
durable request is replayed on startup, and duplicate or late acknowledgements cannot revive a
completed round.

### Editor and history views

The editor creates a valid Loop group (one child Start, one child Agent and their edge). The default
group feeds the child Agent output into the next round's `value`, stops on a non-empty output and
exports it as `result`. Members are added from a member's output port and belong to that Loop;
dragging a node into the frame does not establish ownership. Deleting a Loop removes its descendants
and edges atomically, and root auto-layout keeps child positions.

In a run, the outer path shows each Loop as one container. Opening it shows the first persisted round of its current execution
and an overview; round buttons carry live status, and selecting a round switches the member path,
outputs, inspector and conversation together without borrowing another round's results. Members are
shown in a wrapping grid that follows the definition structure, with unexecuted members in a
disclosure. Condition cards show their IF / ELSE IF / ELSE rules and the decision recorded for that
round. **Loop results** shows the committed exports, error and termination reason, distinguishing a
satisfied condition, an explicit exit and the round limit. Running or failed Loops never present
partial values as exports.

After a restart or ordinary recovery, the round buttons include the container execution ID when rounds from both executions are visible. A debug edit that rebuilds the Loop removes the old rounds from the Theater. Open the run’s debug history to inspect the superseded attempts. Select a round to see its members' execution IDs in the inspector. A
completed historical round shows a specific stop reason only when the recorded container ID matches.

## Execution scopes

Migration `0013` gives every execution an identity so repeated definition node ids do not overwrite
each other.

- `workflow_execution_scopes` stores roots and Loop rounds. A round has a parent Loop node run, a
  one-based index (1–100), a lifecycle status and JSON state; a root has none of these and its state
  stays in the run payload. `workflow_run_root_scopes` names each run's current root.
- `workflow_node_runs.scope_id` assigns every node run to a scope. Inserts without an explicit scope
  attach to the current root; explicit scopes must belong to the same run and current root, and a
  scope never changes. A partial unique index prevents duplicate live definition nodes within a
  scope; a Loop has at most one pending or running round, and each index is unique per parent.
- Restart creates a new root in the transaction that resets the run, so late callbacks from the old
  execution are harmless. Old node runs keep their scopes.
- Downgrading through this migration first appends complete scope and node snapshots to
  `workflow_scope_downgrade_archive`, terminalizes active runs with Loops and hides child node runs
  from older readers. Flat runs, sessions, Workspaces and files are untouched, and the archive
  survives a later re-upgrade as history, not resumable state.

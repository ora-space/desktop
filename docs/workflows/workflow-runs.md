# Workflow Runs

English | [中文](workflow-runs.zh.md)

A workflow run freezes one published snapshot and executes it in a caller-selected Workspace: a
project row targets its Main Workspace, a task row that task's isolated Workspace. Run CRUD lives in
`crates/application/src/workflow_run`, the execution engine in its
[`engine` module](../../crates/application/src/workflow_run/engine/README.md), and agent-node
sessions in `crates/backend/src/workflow/run`. The UI is described in
[`features/workflow-run/README.md`](../../packages/app-shell/src/features/workflow-run/README.md).

## Model

| Domain type | Table | Records |
|---|---|---|
| `WorkflowRun` | `workflow_runs` | The pinned `snapshot_id`, display name, `workspace_id`, kickoff `input`, status and the run payload (variable pool and scheduler state) |
| `WorkflowNodeRun` | `workflow_node_runs` | One executed node attempt with its scope, iteration round, session, output and payload |
| `WorkflowExecutionScope` | `workflow_execution_scopes` | Root executions and Loop rounds, see [Loops and iterations](loops-and-iterations.md#execution-scopes) |

Run and node status share `Pending | Running | Succeeded | Failed | Cancelled`. Nodes that never
started have no row; the frontend derives "not started" by comparing graph nodes with recorded rows.
An interactive node waiting for follow-up input is persisted as `Pending`, and the contract reports
a running run with such a node as `AwaitingInput` so the sidebar can show that a person is needed.

## Lifecycle

| Operation | Effect |
|---|---|
| `createWorkflowRun` | Validate and persist a `Pending` run |
| `startWorkflowRun` | Validate executability and begin scheduling |
| `updateWorkflowRunInput` | Fill Start inputs before the run starts |
| `cancelWorkflowRun`, `restartWorkflowRun` | Cancel; restart from scratch with a fresh root execution |
| `previewWorkflowRunResume`, `resumeWorkflowRunFromFailure` | Inspect and perform a resume, see [Resuming](#resuming-a-failed-run) |
| `completeWorkflowNode` | Manually complete an interactive node |
| `diagnoseWorkflowNodeFailure` | On-demand AI diagnosis of a failed node |
| `getWorkflowRun`, `listWorkflowRuns`, `listWorkflowRunsByWorkflow`, `listWorkflowNodeRuns` | Read runs, project-scoped summaries and node history |
| `renameWorkflowRun`, `deleteWorkflowRun` | Rename; delete a non-active run |

### Creation

`CreateWorkflowRunHandler` admits the requested active Workspace, resolves the snapshot (an explicit
`snapshot_id` or the workflow's published pointer), checks it belongs to the workflow together with
its declared roles and Skill bindings, and persists the run with an empty `current_nodes` anchor. The
frontend passes the exact Workspace of the project or task row that opened the picker; the backend
never infers a branch or creates another worktree.

Creation compiles the run-scoped variable pool from the frozen graph and seeds `{start_id}.input`
with the kickoff text, which defaults to the snapshot's Start instruction. The deploy screen collects
the run name, the instruction and typed values for declared Start inputs. Ora's display locale is
frozen into the run, so every generated handoff uses the same language even if the interface
language changes later.

For each Skill-using node, run creation asks an `AgentSkillDeliveryProvider` for its validated,
Workspace-relative discovery roots and freezes a per-node receipt with the Skill id, slash-command
name and actual package paths. Local Skills resolve through their catalog directory and plugin Skills
through the immutable package recorded in their row. The current provider returns the shared
`.agents/skills` root. Execution uses only the frozen receipt, so catalog changes affect new runs
only.

### Events

After every committed run or node-run transition, including the interactive transitions (an
interactive node parking, a human turn beginning or ending), the backend publishes
`AppEvent::WorkflowRunInvalidated { run_id }`. The event carries no state; run views re-query the
persisted run, and polling (every 1.5 s on the run detail) remains a loss-tolerant fallback. Agent
output streams on the ACP session stream.

### Reading and deleting

Deletion refuses active runs (a `Running` run, a paused run with a non-terminal node, or a `Running`
session bound to one of its nodes) and then soft-deletes the run, its node runs and their sessions,
registering durable Git cleanup where a run owns Git resources. A `Pending` run that never started
can be discarded without cancelling. The Workspace is shared and is never deleted with a run.
Sessions bound to a node run are addressable by id but excluded from ordinary session lists, even
after a restart soft-deleted their rows; they never become ordinary chats. A session bound to a
terminal node is read-only.

## Agent nodes

Before an Agent node is prompted, the backend builds a structured handoff from the frozen graph and
the current node rows: the current step and its direct neighbors, the resolved role as behavioral
constraints, every node in deterministic topological order with its status, and the original run
request kept separate. Predecessor outputs are never appended implicitly; a node receives one only
when its prompt references the variable, such as `{{#agent-1.output#}}`. Enabled Skill slash commands
stay at the start of the first text block (agent CLIs parse them positionally), followed by the
actual Skill package paths in the Workspace, which the agent is told to use. A node's Skill bindings
are required invocations, not an allowlist.

When structured output is enabled, the JSON Schema contract is appended after the workflow context
and constrains only the final assistant response to one bare JSON object, so the agent can still use
tools and edit files during the step.

The node's session uses the node's frozen MCP selection, see
[Session MCP](../agents/session-mcp.md#selection). An enabled MCP dependency that is unavailable
fails the node explicitly; unselected plugins cannot block it.

`workflow_node_runs.output` always stores the final assistant text, including when structured
parsing or validation fails. The session history is the only source of the full conversation.

### Prompt inactivity

`agentConfig.promptInactivity: "wait"` keeps a long-running prompt waiting through silence instead of
the default 45/60/90/120-second windows, see
[Agent runtime](../agents/agent-runtime.md#prompt-inactivity-and-retries). The frozen value applies to
the first turn, node retries, Loop and Iteration rounds and follow-up turns on interactive nodes. A
waiting prompt stays running; it is neither a retry wait nor an interactive pause. A stuck execution
may need a manual stop, and real session errors still follow the node's retry policy. To change an
existing workflow, edit the node, publish a new snapshot and start a new run.

### Interactive nodes

An interactive Agent node parks as awaiting input after its first turn. Follow-up messages run as
human turns on the same session, and `completeWorkflowNode` completes the node manually; a completion
claim prevents a follow-up racing against manual completion. Region members of an Iteration default
to non-interactive and the runtime rejects interactive members.

Each parked turn persists a completion snapshot: its turn key, stop reason, blockers and the final
assistant-text digest. Only an `EndTurn` turn without blockers completes normally. A stopped,
truncated, refused or unknown turn keeps the node awaiting input, and `completeWorkflowNode` rejects
it with `workflow_node_incomplete` (carrying every blocker) unless `force` is requested; a changed
turn or output digest returns `workflow_node_turn_changed` or `workflow_node_output_changed`, naming
the expected and actual identity. The step card at the end of the conversation and the bottom-right
action are the two explicit ways to advance: continue, a confirmed forced completion, or
complete-and-advance when the card is ready.

The opening, workflow-owned turn never reads as ready on its own. Its `EndTurn` only means the
agent finished that reply, which may be a question for the user, so the snapshot carries a
`turn_unconfirmed` blocker and the card keeps the awaiting-input badge with "continue this step"
and "complete anyway (use current output)" (the latter confirms and is audited). The bottom-right
action stays disabled until the user replies or confirms. A turn opened by the user's reply keeps
the rule above: an `EndTurn` without blockers completes normally and the card names the next step.

A turn whose stream dropped before its stop reason became durable parks provisionally from the last
readable history: the snapshot keeps the `userSeq` it describes and a `turn_not_settled` warning, and
a later completion attempt repairs it once the turn settles for that same user message. A node that
parked before completion snapshots existed has no snapshot at all; the first completion attempt
materializes one from the session history and applies the same blockers gate, so an unfinished turn
is refused there too rather than advancing silently.

Every human turn carries a `<workflow_step_boundary>` block rendered from the frozen graph. It names
the current step and its direct successors, and states that only this step is in scope. It records no
promise about the runtime stopping later nodes, and the prompt recorded for the user stays exactly
what the user typed.

A `force` completion is audited in the node payload under `completion_forced`, with the blockers that
were bypassed, whether the node had a settled turn, and where the accepted output came from
(`turn_window` or `last_assistant_message`). The audit is written in the same transaction as the
completion, so a completed node never lacks its record.

## Variables and types

The run's variable pool lives in `workflow_runs.payload`. Its catalog declares typed Start inputs and
stable outputs of data-producing nodes; enabled structured output also declares
`agent-1.structured_output`. `variablePool.values` holds assigned values only. Completed-node writes
commit in the same transaction as the node's status transition. The kickoff instruction stays in
`workflow_runs.input` and is not a selectable variable.

- Condition nodes expose no variables and persist no output; their selected branch is private
  scheduler state (`conditionDecisions`) that survives restarts. They are scope-transparent:
  downstream nodes see the variables of the Condition's direct predecessors.
- For each node the editor offers workflow globals plus variables of every direct predecessor.
  Other ancestors are unavailable unless forwarded. Structured-output field paths appear in the same
  catalog, so selectors never rely on free text.
- Custom globals need an explicit dotted name and a type-correct initial value; system globals are
  runtime-owned.
- Each terminal Output builds its own result object; result names must be unique within that node.

Types are enforced at graph parsing, editor entry and pool writes. `array` and `array[any]` accept
heterogeneous arrays; typed arrays validate every element. `file` is
`{ "kind": "workspace_file", "path": "relative/path" }` and `array[file]` a list of them; absolute
paths, parent traversal, empty and platform-reserved paths are rejected. Structured Agent schemas use
the same types, are validated recursively and generate a valid JSON example in the prompt.

Start inputs keep their form control separate from the pool type: text, paragraph and select produce
`string`, number `number`, checkbox `boolean`, single file `file`, file list `array[file]`, and JSON
any structured type. Choices, required flags and length limits are frozen into the snapshot and
checked again at execution. A rejected value surfaces as `workflow_run_input_invalid` naming the
variable and reason; one submission reports only the first refusal in alphabetical order and applies
nothing.

## Failures

When a node fails, the run fails immediately while in-flight siblings run to completion; the
scheduler then dispatches nothing more. The failed node run records `payload.error_detail`: `kind` (a
mechanical classification, never inferred by a model), `message`, `source_chain`, `attempt`,
`resumable`, `injects_previous_failure`, `auto_retryable` and `recorded_at`.

| Kind | Resumable | Retried automatically |
|---|---|---|
| `session`, `session_ended_without_stop_reason`, `session_binding_rejected` | yes | yes |
| `workflow_model_not_found`, `missing_agent_config`, `interrupted_by_restart`, `repository`, `baseline_persist` | yes | no |
| `structured_output`, `agent_refusal`, `unknown_stop_reason` | no | yes |
| `multiple_outputs` | no | no |
| `prompt_template`, `missing_agent_ref`, `missing_skill_materialization`, `invalid_run_payload`, `condition_evaluation`, `aggregator_no_match` | no | no |

`resumable` predicts whether re-running the same snapshot is a sensible first move; resume is always
offered for a failed or cancelled idle run. Only agent-behavior failures (`structured_output`,
`agent_refusal`, `unknown_stop_reason`, `multiple_outputs`) are injected into the next attempt's
prompt, and only while the run-level `inject_last_failure` switch is on (the default); the rendered
block is stored as `payload.injected_failure_context`.

### Automatic retry

Only Agent nodes retry, including agents inside Iteration regions and Loop bodies, and never
interactive ones. Retry `n` waits `initialDelaySeconds × 2^(n-1)` seconds, capped at 600; the default
policy waits 10 s and 20 s, so a node runs at most three attempts.

A covered failure records the failed attempt's `error_detail`, soft-deletes that attempt and inserts
the next one in the same run, scope and round as a waiting row (`Running`, no `started_at`,
`payload.retry_wait` with attempt numbers, delay and `due_at`). Because the row is `Running`, the run
stays running, independent branches continue and successors wait. When `due_at` passes, a timer
wakes the engine under the run lock, which re-reads the marker and dispatches the attempt in the
scope it failed in; an early or stale wake is a no-op. There is no rollback before a retry, and the
attempt takes its own checkpoint. The silence-triggered prompt resend inside a session is separate;
`"wait"` disables only that, not retries after real failures.

`payload.auto_retry = { retry, max_retries }` counts the budget; a first attempt, a resumed attempt
or one after a restart starts fresh, while `error_detail.attempt` keeps counting (a resume after three
failed attempts runs attempts 4, 5, 6). Inside a Loop both restart in every round. A wait ends early
on cancel (rows become `Cancelled`), on run failure (rows become `Cancelled` with
`{"reason":"retry_abandoned"}`), and on restart (the boot sweep fails an outer waiting row with
`interrupted_by_restart`; a row inside a running Iteration settles its round as failed).

`getWorkflowRun` exposes waiting rows and `failedAttempts`: the earlier soft-deleted `Failed` attempts
with their `error_detail`, oldest first, including the agent's own reason in `sourceChain`. Attempt
numbers can skip, because cancelled or abandoned attempts and attempts without a failure record take a
number but are not listed. The run view shows waiting rows as "Waiting to retry" with a per-second
countdown, lists earlier attempts with how each was replaced (retried automatically, resumed manually,
before "Run again from start"), and notes when a failure kind is never retried.

### Resuming a failed run

Resume soft-deletes the failed or cancelled node runs and all their descendants and reschedules from
the surviving state. Attempts are numbered per `(run, node, iteration)`, and per Loop round for Loop
body rows. The resume unit for anything inside a container is the owning Loop or Iteration, which
restarts from round 1; partial in-container resume is not supported. A run may resume on a newer
published snapshot when the graphs are compatible; removed nodes, changed node types and Start
contract changes are reported (`node_missing:<id>`, `node_type_changed:<id>`, `start_node_changed`,
`start_variables_changed`, `variable_type_changed:<selector>`, `variable_missing:<selector>`), and a
completed Iteration outside the resume unit cannot change its configuration or members. The snapshot a
node last ran is recorded as `payload.snapshot_id`.

### Rollback

Each node records a Git checkpoint under `refs/ora/checkpoints/<node_run_id>` before it runs (see
[Gitlancer](../workspaces/gitlancer.md)), with `checkpoint`, `checkpoint_error` and `file_changes` on
its payload. Resume offers three rollback modes; before rolling back, the Workspace is snapshotted as
`pre-rollback-<run>-<ts>` so the operator can undo.

| Mode | Effect | Unavailable when |
|---|---|---|
| `keep` | Leave the Workspace as is | Never |
| `node_files` | Restore only the paths the failed nodes changed | `no_file_changes`; `composite_region` for a container resume unit |
| `checkpoint` | Restore the whole Workspace to the resume unit's checkpoint | `no_checkpoint`, `siblings_ran_after_checkpoint`, `not_resumable` |

A node replaced by automatic retries rolls back as one unit with every attempt of its retry chain
(`payload.retry_chain`). On-demand AI diagnosis writes `payload.ai_diagnosis`, labelled as a guess and
never read by scheduling, resume, rollback or snapshot switching.

## Recovery after restart

Desktop runs the boot sweep only after it holds the application instance lock (see
[Desktop host](../desktop/desktop-host.md#startup)), so a live second process can never mark running
work as interrupted. Interrupted outer rows fail as `interrupted_by_restart` and fail the run;
interrupted rows inside a running container fail while the container and run survive, and the round
settles as failed. Valid interactive waits are preserved, and tools with unknown side effects are never
replayed automatically.

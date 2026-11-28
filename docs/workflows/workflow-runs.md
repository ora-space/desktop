# Workflow Runs

English | [中文](workflow-runs.zh.md)

A workflow run freezes one published snapshot and executes it in a caller-selected Workspace: a
project row targets its Main Workspace, a task row that task's isolated Workspace. Run CRUD lives in
`crates/application/src/workflow_run`, the execution engine in its
[`engine` module](../../crates/application/src/workflow_run/engine/README.md), and agent-node
sessions in `crates/backend/src/workflow/run`. The UI is described in
[`features/workflow-run/README.md`](../../packages/app-shell/src/features/workflow-run/README.md).

## Model

| Domain type              | Table                       | Records                                                                                                                                 |
| ------------------------ | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `WorkflowRun`            | `workflow_runs`             | The pinned `snapshot_id`, display name, `workspace_id`, kickoff `input`, status and the run payload (variable pool and scheduler state) |
| `WorkflowNodeRun`        | `workflow_node_runs`        | One executed node attempt with its scope, iteration round, session, output and payload                                                  |
| `WorkflowExecutionScope` | `workflow_execution_scopes` | Root executions and Loop rounds, see [Loops and iterations](loops-and-iterations.md#execution-scopes)                                   |

Run and node status share `Pending | Running | Succeeded | Failed | Cancelled`. Nodes that never
started have no row; the frontend derives "not started" by comparing graph nodes with recorded rows.
An interactive node waiting for follow-up input is persisted as `Pending`, and the contract reports
a running run with such a node as `AwaitingInput` so the sidebar can show that a person is needed.

## Lifecycle

| Operation                                                                                  | Effect                                                                              |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| `createWorkflowRun`                                                                        | Validate and persist a `Pending` run                                                |
| `startWorkflowRun`                                                                         | Validate executability and dependencies, then begin scheduling                      |
| `updateWorkflowRunInput`                                                                   | Fill Start inputs before the run starts                                             |
| `cancelWorkflowRun`, `restartWorkflowRun`                                                  | Cancel; restart from scratch with a fresh root execution                            |
| `previewWorkflowRunResume`, `resumeWorkflowRunFromFailure`                                 | Inspect and perform a resume, see [Resuming](#resuming-a-failed-run)                |
| `completeWorkflowNode`                                                                     | Manually complete an interactive node                                               |
| `diagnoseWorkflowNodeFailure`                                                              | On-demand AI diagnosis of a failed node                                             |
| `getWorkflowDebugContext` (`workflowRun.debugContext`)                                     | Read the selected target, effective graph and base revision for editing and preview |
| `getWorkflowRun`, `listWorkflowRuns`, `listWorkflowRunsByWorkflow`, `listWorkflowNodeRuns` | Read runs, project-scoped summaries and node history                                |
| `renameWorkflowRun`, `deleteWorkflowRun`                                                   | Rename; delete a non-active run                                                     |

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

For each Skill-using node, run creation freezes a per-node receipt carrying only `{ node_id,
skill_id, invocation_name }`—never content, plugin version, or paths. At dispatch time, each node
resolves its live formal storage `SKILL.md` path by `skill_id` (local Skills under `<data
root>/atoms/skills/<name>/SKILL.md` and plugin Skills under the row's validated `package_root`),
ignoring any legacy Workspace-relative `packagePaths` frozen by pre-upgrade runs. If a bound Skill
is upgraded mid-run, subsequent nodes read the new version's `SKILL.md`; if uninstalled mid-run, the
dispatched node fails with `workflow_skill_not_found` instead of falling back to another
same-named Skill or stale path.

Before start, restart, or resume dispatches a node, Desktop checks the entire graph it will run
against the current role and Skill catalogs (confirming each `skill_id` resolves to an Available
Skill whose formal storage `SKILL.md` is present and readable) and installed MCP packages. Creation
checks roles and Skills; an enabled MCP binding first needs a valid MCP package at dispatch. A
missing Skill or unavailable MCP fails with `workflow_skill_not_found` or `workflow_mcp_not_found`,
naming the node and dependency. Repairing the dependency allows the same run to proceed. A repeated
start of a running run returns that run; restart of a running run returns
`workflow_run_not_restartable`. Whether an Agent is installed is not part of this precheck: the
runtime resolves the frozen Agent identity when it opens the node's session and fails with
`agent_not_installed` if it is missing.

A resume onto a new snapshot checks the target graph before restoring a checkpoint or changing
the worktree. It freezes `{ node_id, skill_id, invocation_name }` for the target snapshot's Skills
when it switches snapshots. Keeping the current snapshot preserves its existing frozen Skill
receipts. After dispatch, session setup handles an MCP removed or misconfigured before a node
session opens, as well as missing Agent capabilities; it does not deliver a partial selection.

Any Agent can run Skill-bound workflow nodes without declaring Skill delivery capabilities; only
two distinct Skills resolving to the same invocation name within one run are still rejected at
start. For MCP dependencies, an in-flight run's frozen receipt is checked at the next scheduling
operation (`start`, `restart`, or `resume`); for Skills, both pre-start validation and each node
dispatch resolve live storage paths dynamically.

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
live formal storage `SKILL.md` paths resolved at dispatch, which the agent is told to read and
follow. A node's Skill bindings are required invocations, not an allowlist.

When structured output is enabled, the JSON Schema contract is appended after the workflow context
and constrains only the final assistant response to one bare JSON object, so the agent can still use
tools and edit files during the step.

The node's session uses the node's frozen MCP selection, see
[Session MCP](../agents/session-mcp.md#selection). An enabled MCP dependency that is unavailable
fails the node explicitly; unselected plugins cannot block it.

`workflow_node_runs.output` always stores the final assistant text, including when structured
parsing or validation fails. The session history is the only source of the full conversation.

### Prompt inactivity

A workflow Agent configured with `promptInactivity: "wait"` fails its turn after 30 consecutive
minutes without progress. Select unbounded waiting explicitly to disable the silence deadline. Nonempty output, changes to a source’s plan, tool status or output, and completed parent-tool permission replies renew the window. A tool stalled in progress
still reaches the deadline. Silence never resends the prompt. A noninteractive node applies its retry policy after cancellation settles or the old channel is isolated. An interactive first turn fails directly. A human follow-up parks with its session and history. The frozen setting covers all node attempts, loop rounds, and interactive follow-ups.
See [Agent runtime](../agents/agent-runtime.md#prompt-inactivity-and-retries).

### Interactive nodes

Human completion, including force and signal confirmation, uses the attempt ID shown on the run page (`nodeRunId`). If the attempt changes, refresh the run before confirming. Older clients can omit the ID only while the node has one unreplaced attempt in the run.

While a debug edit is stopping, or a failed stop still holds its fence, affected interactive attempts reject follow-up prompts and completion. This includes parked members of every Loop round rebuilt by an upstream edit. The fence also prevents completion-snapshot repair. A completion claimed before the edit checks the attempt again before committing. A rejected completion returns `workflow_node_not_awaiting_input`.

An interactive Agent node parks as awaiting input after its first turn. Follow-up messages run as
human turns on the same session, and `completeWorkflowNode` completes the node manually; a completion
claim prevents a follow-up racing against manual completion. Region members of an Iteration default
to non-interactive and the runtime rejects interactive members.

Each parked turn persists a completion snapshot: its turn key, stop reason, blockers and the final
assistant-text digest. A new or re-saved interactive Agent node defaults to
`completionSignal: agent`; the host injects a loopback completion tool into that node's session, and
the node completes only after the agent calls the tool for the settled attempt and the user confirms
(or the node sets `autoAcceptAgentCompletion`). A turn without a signal carries
`agent_signal_pending`; an accepted signal without a confirmation carries
`agent_confirmation_pending`; both are written into the parked snapshot beside the other blockers.
`completionSignal: end_turn` legacy graphs keep the old rule: only an `EndTurn` turn without
blockers completes normally. A stopped, truncated, refused or unknown turn keeps the node awaiting
input, and `completeWorkflowNode` rejects it with `workflow_node_incomplete` (carrying every blocker)
unless `force` is requested; a changed turn or output digest returns
`workflow_node_turn_changed` or `workflow_node_output_changed`, naming the expected and actual
identity. The step card at the end of the conversation and the bottom-right action are the two
explicit ways to advance: continue, a confirmed forced completion, or complete-and-advance when the
card is ready. A confirmation names the `signalId` the user saw; a stale id returns
`workflow_node_signal_changed` instead of confirming a newer turn. A failed attempt, a completed node
and a cancelled run all revoke their attempt tokens, and a restarted Desktop restores the completion
tool with the persisted session.

If an automatic retry ends without a usable result from its final attempt, the step shows
**Node execution failed**. Normal and automatic completion are blocked, and dependent nodes stay
idle. Choose **Retry this step** to continue in the same conversation. A successful recovery
retains the earlier failure in the audit. Confirming a forced completion records the override
in `completion_forced.retry_reason`. The warning is `retry_without_result`, and the failure
remains in `payload.promptFailures` after recovery. Parking, normal completion, automatic
acceptance and forced completion all use the same final-attempt output window.

A node may declare 1–32 workspace-relative deliverables in `agentConfig.completion.files`, using paths or the minimal glob set: `*` (within one segment), `**` (across directories) and `?`. Character classes, braces, escaping and negation are rejected; matching is case-sensitive with no Unicode normalization, and directories, gitlinks and symbolic links that escape the workspace never count. A graph declaring `completion` must use `agentConfig.schemaVersion: 4`; version 3 with `completion`, a version above 4, or undefined fields such as `criteria` or `checklist` are rejected by graph parsing instead of being dropped. The inspector writes version 4 when the first file is added and clears the declaration, returning to version 3, when the last file is removed.

The file check runs against the live workspace on every completion click and is never written into `payload.completion`: at least one matching regular file must exist, and `mustChange: true` additionally requires one match to differ from the worktree baseline persisted when the node started. Missing and unchanged declarations return `file_missing` / `file_unchanged`, with the original declaration in the blocker's `params.path`; after fixing the workspace the user can retry directly without re-parking, and `force` overrides the blockers while recording them in `completion_forced`. When the baseline is missing or corrupt, existence is still checked and `mustChange` degrades to the `file_check_unavailable` warning, which is visible in the completion projection and survives completion. Existence also falls back to a contained walk of actual workspace files when Git ignores a path or the content snapshot exceeds its budget, and a legacy baseline that recorded a binary file as `null` produces the same warning instead of claiming a change or no change.

The opening, workflow-owned turn never reads as ready on its own. Its `EndTurn` only means the
agent finished that reply, which may be a question for the user, so the snapshot carries a
`turn_unconfirmed` blocker. The step card offers "confirm and complete" — a normal completion sent
with `confirmInitialTurn`, never audited as forced — beside "continue this step", and the
bottom-right action stays disabled until the turn is ready. A turn opened by the user's reply keeps
the rule above: an `EndTurn` without blockers completes normally and the card names the next step.
For an `agent`-policy opening turn with a signal, the confirmation must carry both
`confirmInitialTurn` and `confirmAgentCompletion`; one field cannot lift the other gate. Without a
signal, the user can still advance through the second-confirm path, which sends
`confirmInitialTurn` and `force` together and writes a forced audit; `force` alone is rejected with
`turn_unconfirmed`. Any other request that sets both fields is rejected as `invalid_request`.

Every park is classified where it is written. `parkKind` is `awaiting_user` for an unconfirmed
opening turn, a turn waiting for the agent signal or the user's confirmation, the user's own stop,
or a provisional snapshot, and `abnormal` for a timeout,
truncation, refusal, unknown end, session crash, stream drop, or output-contract failure; `stopCause`
names why the owning prompt ended (`user_stopped`, `prompt_timed_out`, `stream_dropped`,
`session_crashed`, `unknown`). The card shows an abnormal stop with its reason and offers only
continue or a confirmed forced completion; the bottom-right action is enabled only for a ready turn.
Older snapshots without these fields keep their previous paused/unknown display, and a repaired
provisional snapshot keeps the turn origin and stop cause it was parked with.

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

A `force` completion is audited in the node payload under `completion_forced`, with `by: "human"`,
the effective blockers it covered, the stop cause that explains them, whether the node had a settled
turn, and where the accepted output came from (`turn_window` or `last_assistant_message`). The audit
is written in the same transaction as the completion, so a completed node never lacks its record.

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

| Kind                                                                                                                                          | Resumable | Retried automatically |
| --------------------------------------------------------------------------------------------------------------------------------------------- | --------- | --------------------- |
| `session`, `session_ended_without_stop_reason`, `session_binding_rejected`                                                                    | yes       | yes                   |
| `workflow_model_not_found`, `missing_agent_config`, `interrupted_by_restart`, `repository`, `baseline_persist`                                | yes       | no                    |
| `structured_output`, `agent_refusal`, `unknown_stop_reason`                                                                                   | no        | yes                   |
| `multiple_outputs`                                                                                                                            | no        | no                    |
| `prompt_template`, `missing_agent_ref`, `missing_skill_materialization`, `invalid_run_payload`, `condition_evaluation`, `aggregator_no_match` | no        | no                    |

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

| Mode         | Effect                                                      | Unavailable when                                                  |
| ------------ | ----------------------------------------------------------- | ----------------------------------------------------------------- |
| `keep`       | Leave the Workspace as is                                   | Never                                                             |
| `node_files` | Restore only the paths the failed nodes changed             | `no_file_changes`; `composite_region` for a container resume unit |
| `checkpoint` | Restore the whole Workspace to the resume unit's checkpoint | `no_checkpoint`, `siblings_ran_after_checkpoint`, `not_resumable` |

A node replaced by automatic retries rolls back as one unit with every attempt of its retry chain
(`payload.retry_chain`). On-demand AI diagnosis writes `payload.ai_diagnosis`, labelled as a guess and
never read by scheduling, resume, rollback or snapshot switching.

## Editing and resuming from a selected execution

Select an execution or an undispatched position in the Theater inspector and choose **Edit and resume from here**. Edit the effective configuration, choose **Preview changes**, and review what will be kept, stopped, invalidated and scheduled. Choose **Confirm and resume** to apply the edit. The receipt stays visible until you close the editor. If the reply is lost, retry in the same editor to reuse the original request ID, or check the run’s debug history. If the selection moves while you are drafting, the editor keeps your changes for the original execution. Return to that position to preview them. If the selection changes after preview, close the editor, select a position and preview again. An open editor stays visible when the run finishes. If the server rejects a stale preview, the editor reloads the current configuration for a fresh preview.

## Querying run-local debug history

Open **Debug history for this run** on the run page to inspect accepted operations, their impact,
and their patches. Expand an execution ID to read the configuration recorded at dispatch. A waiting
retry can link to the earlier execution that supplied its input. For a position selected before
dispatch, history links to the first execution once the scheduler creates it. The Theater inspector
also opens the selected execution record.

Editing a while member resumes at the selected one-based round. Earlier rounds and unaffected members keep their results. Subsequent rounds of this Loop execution use the revised configuration and new feedback; superseded attempts remain queryable by id. Editing the Loop configuration instead starts a new container execution at round one. Preview rejects a member edit that would leave the round's explicit exit in effect.

To edit the Condition or another node that led to an explicit exit, wait for the cancelled siblings to stop. Once cleanup succeeds, you can preview the edit even if the round failed because a cancelled sibling never produced an exported value. The preview includes the siblings that will reopen and their reachable successors. If cleanup failed, or an older failed round has no cleanup receipt, use ordinary failure recovery.

If the run is cancelled before the acknowledgement arrives or cleanup fails, Desktop cannot verify that cancelled siblings stopped. Previews that would retire that exit, including edits upstream of the Loop, remain blocked. Use [ordinary failure recovery](#resuming-a-failed-run) to restart the affected Loop from round one.

A waiting automatic retry can preview the nearest recorded dispatch in its retry chain at the same node, scope, container attempt, iteration, snapshot and revision. The preview names that attempt in `inputSourceNodeRunId`. If that nearest dispatch differs in any coordinate, preview is refused even when an older dispatch matches. Missing dispatch evidence also prevents preview.

`workflowRun.debugHistory({ runId })` returns confirmations in acceptance order with the request key, preview, patch and phase. An applied edit identifies its parent and resulting revisions. A failed stop retains its reason and the attempts that could not be proven stopped (`unsettledNodeRunIds`). Each entry also records the status and error the run had when it was accepted (`priorRunStatus`, `priorError`): a failed, cancelled or succeeded run keeps them while the stop is in flight and returns to `running` in the commit that applies the revision. After reconnecting, the request key identifies the same operation. While a stop is in progress or failed without a released fence, new previews, new confirmations, failure recovery, a whole-run restart and a snapshot switch are refused with `workflow_debug_operation_active`. Refusals reach the page as typed public errors: `workflow_debug_stale_preview`, `workflow_debug_operation_active`, `workflow_debug_request_conflict`, `workflow_debug_patch_invalid` with the field path and reason, and `workflow_debug_target_rejected` with the missing fact or position.

`workflowRun.debugAttempt({ runId, nodeRunId })` also reads superseded attempts. It returns the round or item, configuration revision, outcome and availability of recorded input. The third foreach item has `index=2`, and the third while round has `round=3`. Pre-upgrade attempts without a recorded association return null configuration and revision.

A debug preview selects an existing attempt by `nodeRunId` or an undispatched position by `pendingTarget: { rootScopeId, scopeId, nodeId }`. A pending foreach member also requires `containerNodeRunId` and its zero-based `index`. Preview checks the scheduler’s ready projection. If normal dispatch wins before confirmation, refresh the view and select the recorded attempt. After the revision applies, the history entry names the attempt the scheduler first admitted for that position in `firstNodeRunId`; it stays null until then. An independent sibling can advance while the target and its dependencies stay valid. Container previews include the member configurations that will apply at the new entrance. The Iteration source accepts only a variable that is declared as an array at the root; a path into a nested array is rejected on `iteratorSelector`.

Editing an Agent or Condition in a foreach item restores the input recorded for that item. Earlier items keep their results. Within the selected item, predecessors keep their results too. The edited configuration applies to that member for the rest of this container execution. Each replayed item replaces its contribution to the ordered entries, collected output and failure count. Editing the Iteration source, collection selector or error strategy starts a new container execution from its entrance.

Failure recovery and a whole-run restart take the run’s current revision as their configuration baseline. A restart builds a new root and top-level edits stay in effect; a container that either entry rebuilds starts from its entrance configuration, and overrides bound to the previous container execution remain history only. Only a successful snapshot switch starts a new baseline: it commits the new snapshot together with a new run revision, earlier patches and attempts stay readable as history and are not replayed, also when the run returns to a snapshot it used before, and previews taken before the switch go stale. A refused switch keeps the snapshot and the revisions.

## Recovery after restart

If an independent branch fails while a foreach member edit is stopping downstream work, the run keeps its failure and its last visible aggregate. On ordinary recovery the original container replays from the selected item. Downstream consumers wait for the rebuilt aggregate.

If the old attempts have stopped but the confirmed foreach edit cannot restore its inputs, Desktop rolls back the edit, records the reason as `stop_failed`, and releases the dispatch fence in the same transaction. A running workflow moves to Failed with that reason. An already finished workflow keeps its outcome. No new attempt starts automatically. Use ordinary failure recovery to continue from the last effective revision.

If you cancel while a confirmed debug edit is stopping old work, its receipt records `stop_failed` with reason `run_cancelled`. A running workflow is cancelled; a previously finished workflow keeps its original outcome. If the edit applied first, its revision stays in history, but cancellation prevents the remaining dispatches.

Once the edit applies, every retired attempt rejects completion with `workflow_node_not_awaiting_input`. A follow-up prompt requires a session bound to a current interactive attempt. Open the replacement attempt to continue.

On startup Desktop checks unfinished debug operations before ordinary interruption cleanup. If isolation of an external writer cannot be proved, history lists the unresolved attempt IDs and their scope remains blocked. If the revision was already applied, the scheduler can dispatch a ready, unclaimed position. An attempt already started under that revision follows ordinary interruption recovery; it is not dispatched a second time.

Desktop runs the boot sweep only after it holds the application instance lock (see
[Desktop host](../desktop/desktop-host.md#startup)), so a live second process can never mark running
work as interrupted. Interrupted outer rows fail as `interrupted_by_restart` and fail the run;
interrupted rows inside a running container fail while the container and run survive, and the round
settles as failed. Valid interactive waits are preserved, and tools with unknown side effects are never
replayed automatically.

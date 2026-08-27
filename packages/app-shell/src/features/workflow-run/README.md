# workflow-run

Product UI for **graph workflow runs** executed inside project Workspaces
(sibling to task projections in the workspace tree). Host/Run ports and the memory mock engine live in
[`@ora/workflow-runtime`](../../../../workflow-runtime/README.md).

## Three surfaces (D5.2 boundaries)

Keep these stacks separate — shared chrome only where noted.

1. **Workflow editor** — definition authoring and publishing only.
   Owns catalog / reconnect / delete and the library graph (sidebar list +
   canvas). It does not choose execution location or create runs.
2. **OpenSpec stepper + `workflow-store`** — Spec-mode composer workflow.
   Must **not** write `GraphWorkflowRun` or share run state with Theater.
3. **This module (`GraphWorkflowRun` Theater / Overview)** — a run bound to the
   Workspace selected from a project or Task row. Consumes `@ora/workflow-runtime` via React
   context; owns Theater / Overview / hooks only.

|          | Workflow editor             | OpenSpec / `workflow-store` | `workflow-run`                 | `@ora/workflow-runtime`      |
| -------- | --------------------------- | --------------------------- | ------------------------------ | ---------------------------- |
| Owns     | Definition edit and publish | Spec stepper state          | Run creation + Theater context | Ports, memory engine, events |
| Must not | Drive live run Theater      | Mutate `GraphWorkflowRun`   | Reuse editor `WorkflowCanvas`  | Own React / Theater          |

## Responsibilities

- Wire project mounts and `GraphWorkflowRun` lists through react-query hooks
  against the injected `WorkflowRuntime`.
- Render the Run Workspace when `workflowRunId` is selected:
  - **Theater**: focused act stage + path rail. The path rail
    (and parallel act lists) use a derived order: topological constraints first,
    then canvas position (`x`, then `y`, then id) among concurrently ready
    nodes — not the frozen snapshot array order. Parallel
    `running` / `awaiting_input` nodes share a drag-to-switch stage carousel
    (chips + arrow keys for precise jumps). A soft right **act inspector**
    (settings-parity) opens as a right **overlay** from the stage card click
    (or a new outcome) so the centered act card does not shift. Configuration
    fields are always **read-only** (settings-parity, including Agent model /
    role / enabled skills / prompt). Role/skills with catalog descriptions and
    long instruction/prompt text open a brief preview popover. The rail stays
    resizable/collapsible — drag the handle to resize, drag narrow or use the
    close control to dismiss. Switching acts updates the open panel.
    The focused stage card owns a **session dock**. Without HITL it sits
    lower-right beside the clock range. With embedded HITL it joins the gate chrome so
    permission / clarify and conversation share one action cluster. Activating it
    keeps the card shell/header stable and morphs the body into a compact node
    conversation that reuses the full chat bubble and Markdown renderer.
    Completing an interactive node keeps its action visibly busy through the
    backend transition and run-detail refresh, then moves the open dock to the
    first active successor in the same stable path order. If a completion
    releases parallel successors, the earliest carousel/path item opens.
    An open non-interactive session follows the same successor selection when
    its running node reaches a terminal state, without requiring an action.
    User input and formal Agent messages stay visible; thoughts and tool calls
    remain available behind one collapsed activity disclosure. The inspector
    therefore does not duplicate runtime input/output; it remains the
    configuration, error, and artifact surface.
  - **Result act**: when the run is terminal and Theater focus is not pinned
    to a path node (`focusNodeId === null`), the stage shows an end-of-run
    result surface (status, changed-file count, Overview CTA). Finishing
    clears focus so the result act is the default; path chips (and the Result
    chip) still open/close single-node review, and that pin survives Overview ↔
    Theater. Cold-open of a finished run still primes **Overview**. Run again
    stays in the header.
  - Status language (keep it harmonious):
    - **Working / running**: sky spinner + soft breathe on the card badge/frame
      only (stage focus or Overview node).
    - **Waiting (`awaiting_input`)**: amber mark / badge / path chip (same warm
      “must handle” cue as the HITL prompt). Path progress sheen turns amber while
      any gate is open. Clicking a waiting path chip focuses that act; HITL embeds
      in the focused card (see HITL below). Stage content uses safe vertical
      centering (`my-auto`) so tall HITL stacks scroll from the top and never
      cover the path rail.
    - **Terminal**: result act by default; one check / x / triangle on card
      badges when reviewing a history act. Quiet path/header marks stay dots
      (partial_failed uses a small triangle so it is not identical to failed).
    - **Quiet dots**: path chips, run header, inspector, idle/pending — pure
      color, no pulse, no spinner (except partial quiet triangle).
    - Progress track picks a terminal tint (emerald / rose / muted); sheen is
      the only ambient motion while live.
- Keep OpenSpec composer stepper (`features/workflow` + `workflow-store`) and
  settings React Flow editor interaction out of this module (shared chrome only).

## Non-responsibilities

- Does not own Host/Run repository implementations (see `@ora/workflow-runtime`).
- Does not persist definitions in `@ora/workflow-mock` (that package stays
  session-demo + validation).
- Does not own OpenSpec Spec-mode state.
- Does not own Task Diff rendering (reuses `WorkspaceReviewLayout` /
  `TaskDiffView` from chat); the run review surface is scoped to its Workspace
  and does not infer ownership through a Task.
- Does not implement session-scoped Diff for Theater stage mode yet.
- Does not reuse editor `WorkflowCanvas` (no catalog / reconnect / delete).
- Does not implement HITL timeout (always waits for submit; `HitlTimeoutPolicy`
  enum reserved for later).
- Does not aggregate `partial_failed` statistics (UI copy placeholder only).
- Kickoff remains optional free text on create; schema Kickoff UI can reuse
  `WorkflowFieldForm` later.

## Workspace run invariant

- Every picker selection creates a new run against one explicit `workspaceId`.
- Project rows target their Main Workspace; Task rows target their Isolated Workspace.
- Run creation never asks for a branch and never provisions another worktree.

## Interactions

- Run workflow (sidebar): the project or Task plus menu freezes the target
  Workspace before opening the workflow picker, then creates and selects the run.
- Selection: `useWorkspaceSelectionStore.selectWorkflowRun`.
- **Changes / Diff**: the run workspace wraps Theater and Overview in the same
  `WorkspaceReviewLayout` used by chat. The run owns no implicit Task or
  worktree, so the current generic Workspace review surface does not infer a
  task diff. Stage-scoped Diff is deferred until a session-level Git Diff API
  (or turn-level filter) exists; `nodeStates.sessionId` is projected for that
  follow-up.
- **Open location**: the run header reuses `LocationActionsButton`
  (File Manager / Terminal / VS Code / Copy Path). It resolves the run's
  Workspace location directly; non-local Workspace adapters remain responsible
  for providing a future remote surface.
- Lists: react-query via `queryKeys.workflowMounts` /
  `workflowMountsByDefinition` / `workflowRuns`.
- Runtime: `WorkflowRuntimeProvider` in `AppShell` injects
  `@ora/workflow-runtime` (`createMemoryWorkflowRuntime` today).
  The Desktop AppShell can supply `AppShell.workflowRuntime`; the future
  production adapter will wrap the repository-wide generated contracts client
  over Tauri commands and channels.
  `useGraphWorkflowRunLiveSync` patches run caches via `runs.watch`.
  Per-run UI side effects (artifacts cache, HITL toast, result-act focus)
  share one cursor-aware `runs.subscribe` inside `useGraphWorkflowRunLive`.
  The initial run/artifact snapshot and cursor are fetched atomically so events
  emitted during setup are replayed instead of being lost or overwriting cache.
  Sidebar supports cancel (keep row) and delete (cancel then remove).
- View toggle: Theater ↔ Overview. Overview node click returns to Theater
  focused on that node and opens the act inspector when the stage area is
  wide enough (≥1000px); narrow windows skip the auto-open so the act card
  stays readable. Header Theater toggle does not force the rail open.
  Opening Changes/Files does not auto-open the act inspector (and seeded
  Overview → Theater opens wait until Diff is closed). Clicking an act card
  while Diff is open still opens the inspector beside Diff — they are not
  mutually exclusive. Clicking Overview again while already there
  re-runs `fitView` and re-enables resize auto-fit after a manual pan/zoom.
  While Overview stays open, pane/window resize debounces a `fitView` so the
  graph stays framed — until the user pans or zooms, which pauses auto-fit.
  `awaiting_input` does **not** force a view change (toast only); warm
  overview/path chips stay discoverable in place.
- Theater focus: a live pin (focused while `running` / `awaiting_input`)
  releases back to auto-follow when that same act just finishes. Clicking a
  already-finished or idle node is a history pin and stays until the user
  picks another. On `run_finished`, focus clears in every view so the
  **result act** is the default Theater landing (no leftover live pin when
  finishing on Overview). Finishing while on Overview also shows a toast CTA
  to open the result act. Overview ↔ Theater keeps an explicit post-run pin;
  path rail appends a **Result** chip after the nodes (status-toned like the
  progress track: emerald / rose / zinc) and a second header Theater click
  while already reviewing to return to the result act. Overview node click still
  opens Theater on that pin. Terminal Overview with no pin does not paint a
  fallback node as selected.
- Outcomes / config: `useGraphWorkflowRunLive` lists + patches on
  `artifact_added`. Theater scopes them in the act inspector with the focused
  node; each reveal focuses that act **once** when the stage is not already
  there (does not re-pin or re-open the inspector on the producing act — that
  tween was flashing the card). Overview shows a per-node count affordance only.
- HITL: mock `prompt` nodes pause with `awaiting_input` and append to
  `openHitls` (`kind` + optional `prompt` + `blocking` + field schema). Model
  questions use `kind: "clarify"` with `prompt` shown in the dock. First gate
  discovery expands HITL only when the stage is already on that waiting act;
  on another card the under-stage prompt stays collapsed until opened. Mid-run
  toast still does not force Overview → Theater. Expanded HITL and the
  act inspector are mutually exclusive. Waiting acts **embed** HITL in the card
  footer (warm collapsed prompt → question body + tiles / composer), scoped to
  **that node only** — no multi-gate tabs or “N nodes waiting” copy inside the
  card; peer switches use the parallel dots / name chips under the stage. If
  focus is on a non-waiting act while other gates are open, a compact under-stage
  prompt may list every open gate so the user can jump (expanded HITL collapses
  when focus leaves a waiting act). Collapse is respected across later run ticks.
  Esc collapses HITL first; a second Esc returns Overview. Submit payload keys
  match `field.name`. Text / textarea fields reuse the chat `ComposerEditor`
  (Enter submits, Shift+Enter newline) without chat attachments or `@` / `/`
  menus. Drafts store `documentPlainText` and reload as the same nodes so a
  Theater remount does not flash raw `**` / `#` markers. The focused card
  conversation shows node input, Agent
  messages, and submitted HITL answers (mock approval gates also append a short
  assistant ack so the session visibly updates after submit); the inspector
  remains available without duplicating that transcript.
- Stop confirm: if the run reaches a terminal status while the dialog is open,
  the dialog dismisses (and Confirm is a no-op close) so a finished run cannot
  leave a stuck modal after `preventDefault` on the action button.

## Node conversation integration

- `WorkflowRunLiveSnapshot.conversation` remains a lightweight presentation projection used for
  run-level indicators. Opening a stage card resolves the node state's opaque `sessionId`, loads
  that session through the shared chat store, and renders the ordinary `ChatView` transcript.
- The projection follows the formal chat stream contract: backend adapters must
  deliver one run's events in cursor/sequence order and keep item IDs stable.
  The frontend applies incremental id-based upserts and does not globally re-sort
  conversation items.
- Each node state exposes an opaque `sessionId`; conversation items repeat that identity for
  projection correlation, while the stage session surface uses the identifier directly.
- The memory engine creates deterministic node session IDs, mock user/Agent
  messages, and collapsed thought/tool activity. Production adapters can feed
  raw activity into the projection without exposing it by default.
- Interactive and non-interactive agent nodes share the same session loading and transcript
  rendering. Only interactive nodes expose the composer and explicit completion action; read-only
  nodes retain the return-to-summary action without creating a second conversation renderer.
- A running interactive node stops through the ordinary Session prompt-cancellation control, so
  the workflow-owned automatic first turn and later user turns have the same UI semantics. The
  cancelled Session remains open and the node returns to awaiting input.
- Production binds a node's Session before preparing its automatic prompt so cancellation can
  always clean it up. While that node remains running, the dock retries an empty attachment-seeded
  replay until the first prompt appears, so automatic successor navigation cannot become stuck on
  the initial chat surface.
- Only the focused stage card can enter conversation mode. Parallel peers keep
  stable carousel geometry. The session dock stays available while embedded HITL
  is open: it sits inside the HITL action cluster (collapsed: beside the warm
  prompt; expanded: next to collapse) with matching amber chrome so gate and
  conversation read as one footer composition. Wheel scrolling past the
  conversation edge chains into the Theater stage scroll so HITL below can be
  reached without moving the cursor. Open conversation is keyed by node id in
  the workspace so Overview ↔ Theater remounts restore the same session view.
  Opening a session also pins that act and wins over auto-follow: live-pin
  release and artifact reveal cannot steal the stage, and Theater always shows
  the session node until the reader closes the dock, picks another path node, or
  an automatic node finishes and advances the dock to its first active successor.
  Node conversation reuses `MessageBubble` / `MarkdownMessage` **outside**
  task `MessageList`, so it does not receive chat inline artifact links. Those
  links belong to the task review chat, not the Theater card.

## Demo path checklist

Manual smoke after creating a run (mock runtime; no browser e2e required):

1. Project/Task plus → Run workflow → sidebar shows a new Run.
2. Start → Theater advances along the path.
3. Outcomes appear → act inspector / path badge counts.
4. Prompt node HITL → submit and continue.
5. Stop (cancel) **or** let the run finish → result act on Theater.
6. Header **Run again** creates a fresh pending run.

# workflow-run

Product UI and mock runtime for **graph workflow runs** attached to projects
(sibling to tasks in the workspace tree).

## Three surfaces (D5.2 boundaries)

Keep these stacks separate — shared chrome only where noted.

1. **Settings React Flow editor** — definition authoring and **Deploy to project**.
   Owns catalog / reconnect / delete and the library graph. Not this module’s
   canvas.
2. **OpenSpec stepper + `workflow-store`** — Spec-mode composer workflow.
   Must **not** write `GraphWorkflowRun` or share run state with Theater.
3. **This module (`GraphWorkflowRun` Theater / Overview)** — project-level
   **run** workspace after deploy. Mock Host/Run repositories + Theater UI.

| | Settings RF | OpenSpec / `workflow-store` | `workflow-run` |
| --- | --- | --- | --- |
| Owns | Definition edit, deploy entry | Spec stepper state | Mounts, runs, Theater |
| Must not | Drive live run Theater | Mutate `GraphWorkflowRun` | Reuse settings `WorkflowCanvas` |

## Responsibilities

- Host project mounts of workflow definitions and create/list `GraphWorkflowRun`
  instances (mock Host/Run repositories today; shape ready to extract).
- Render the Run Workspace when `workflowRunId` is selected:
  - **Theater**: focused act stage + path rail + live totals. Parallel
    `running` / `awaiting_input` nodes share a drag-to-switch stage carousel
    (chips + arrow keys for precise jumps). A soft right **act inspector**
    (settings-parity) opens as a right **overlay** from the stage card click
    (or a new outcome) so the centered act card does not shift. While the run
    is `pending`, description / instruction are editable on the **run
    snapshot only** (label「仅本次」); after start they lock to read-only.
    The rail stays resizable/collapsible — drag the handle to resize, drag
    narrow or use the close control to dismiss. Switching acts updates the
    open panel.
  - **Result act**: when the run is terminal and Theater focus is not pinned
    to a path node (`focusNodeId === null`), the stage shows an end-of-run
    result surface (status, totals, artifact count, Overview CTA). Finishing
    while already on Theater clears focus so the result act appears; path
    chips still open single-node review. Cold-open of a finished run still
    primes **Overview**. Run again stays in the header.
  - Status language (keep it harmonious):
    - **Working / running**: sky spinner + soft breathe on the card badge/frame
      only (stage focus or Overview node).
    - **Waiting (`awaiting_input`)**: amber mark / badge / path chip (same warm
      “must handle” cue as the HITL prompt). Path progress sheen turns amber while
      any gate is open. Clicking a waiting path chip focuses that act; HITL docks
      on / under the stage (see HITL below). Stage content uses safe vertical
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

- Does not persist definitions in `@ora/workflow-mock` (that package stays
  session-demo + validation).
- Does not own OpenSpec Spec-mode state.
- Does not call Rust/contracts workflow APIs yet.
- Does not reuse settings `WorkflowCanvas` (no catalog / reconnect / delete).
- Does not implement HITL timeout (always waits for submit; `HitlTimeoutPolicy`
  enum reserved for later).
- Does not aggregate `partial_failed` statistics (UI copy placeholder only).
- Kickoff remains optional free text on create; schema Kickoff UI can reuse
  `WorkflowFieldForm` later.

## Mount vs run (product invariant)

- **Mount**: at most one `(projectId, definitionId)`. Remount refreshes the
  stored definition snapshot. Many projects may mount the same definition.
- **Run**: every successful deploy creates a **new** `GraphWorkflowRun` under
  the project (sidebar lists runs, not mounts).
- First deploy = mount + first run; later deploy to the same project = refresh
  mount + another run (UI copy distinguishes the two).

## Interactions

- Deploy (settings): searchable project picker, then mount upsert + create
  run, select that run, and close settings. Kickoff input belongs in the main
  workspace UI later (`create` / path policy already accept `kickoffInput`).
- Selection: `useWorkspaceSelectionStore.selectWorkflowRun`.
- Lists: react-query via `queryKeys.workflowMounts` /
  `workflowMountsByDefinition` / `workflowRuns`.
- Runtime: `WorkflowRuntimeProvider` in `AppShell` (memory + mock engine).
  `useGraphWorkflowRunLiveSync` patches run caches via `runs.watch`.
  Sidebar supports cancel (keep row) and delete (cancel then remove).
- View toggle: Theater ↔ Overview. Overview node click returns to Theater
  focused on that node and opens the act inspector. Header Theater toggle
  does not force the rail open. `awaiting_input` does **not** force a view
  change (toast only); warm overview/path chips stay discoverable in place.
- Theater focus: a live pin (focused while `running` / `awaiting_input`)
  releases back to auto-follow when that same act just finishes. Clicking a
  already-finished or idle node is a history pin and stays until the user
  picks another. On `run_finished` while Theater is open, focus clears so the
  **result act** shows. The header Theater toggle on a terminal run also
  clears focus so re-entry lands on the result act (Overview node click still
  keeps an explicit pin).
- Stop confirm: if the run reaches a terminal status while the dialog is open,
  the dialog dismisses (and Confirm is a no-op close) so a finished run cannot
  leave a stuck modal after `preventDefault` on the action button.
- Outcomes / config: `useGraphWorkflowArtifacts` lists + patches on
  `artifact_added`. Theater scopes them in the act inspector with the focused
  node; reveal expands the rail and focuses that act when already on stage.
  Overview shows a per-node count affordance only.
- HITL: mock `prompt` nodes pause with `awaiting_input` and append to
  `openHitls` (`kind` + optional `prompt` + `blocking` + field schema). Model
  questions use `kind: "clarify"` with `prompt` shown in the dock. On Theater,
  the waiting act card can **embed** the expandable HITL surface (warm collapsed
  prompt → sky pulse + question body + tiles / composer) in place of metrics —
  no absolute bottom overlay covering the card. If focus is on a non-waiting
  act while other gates are open, a compact prompt sits under the stage column.
  Parallel waits dock HITL **under** the carousel (not inside sliding cards) so
  peer switches stay height-stable. Collapse is respected
  across run ticks so you can browse other nodes while a gate waits — including
  completed / pending path chips during parallel waits (stage leaves the
  parallel carousel when focus is outside the live peer set). Esc
  collapses HITL first; a second Esc returns Overview. Submit payload keys
  match `field.name`. Inspector shows per-node runtime `input` / `output`
  summaries when the mock (or backend) provides them.

## Demo path checklist

Manual smoke after deploy (mock runtime; no browser e2e required):

1. Settings → Deploy to project → sidebar shows a new Run.
2. Start → Theater advances along the path.
3. Outcomes appear → act inspector / path badge counts.
4. Prompt node HITL → submit and continue.
5. Stop (cancel) **or** let the run finish → result act on Theater.
6. Header **Run again** creates a fresh pending run.

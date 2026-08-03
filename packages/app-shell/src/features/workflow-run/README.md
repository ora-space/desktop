# workflow-run

Product UI and mock runtime for **graph workflow runs** attached to projects
(sibling to tasks in the workspace tree).

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
  - Status language (keep it harmonious):
    - **Working** (`running` / `awaiting_input`): spinner + soft breathe on the
      card badge/frame only (stage focus or Overview node). Same rule both views.
    - **Terminal**: one check / x on the card badge (not on path / header /
      inspector echoes).
    - **Quiet dots**: path chips, run header, inspector, idle/pending — pure
      color, no pulse, no spinner.
    - Progress sheen is the only ambient run-level motion.
- Keep OpenSpec composer stepper (`features/workflow` + `workflow-store`) and
  settings React Flow editor interaction out of this module (shared chrome only).

## Non-responsibilities

- Does not persist definitions in `@ora/workflow-mock` (that package stays
  session-demo + validation).
- Does not own OpenSpec Spec-mode state.
- Does not call Rust/contracts workflow APIs yet.
- Does not reuse settings `WorkflowCanvas` (no catalog / reconnect / delete).
- HITL forms arrive in a later step (events already exist).

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
  does not force the rail open. `awaiting_input` forces Theater.
- Theater focus: a live pin (focused while `running` / `awaiting_input`)
  releases back to auto-follow when that same act just finishes. Clicking a
  already-finished or idle node is a history pin and stays until the user
  picks another.
- Stop confirm: if the run reaches a terminal status while the dialog is open,
  the dialog dismisses (and Confirm is a no-op close) so a finished run cannot
  leave a stuck modal after `preventDefault` on the action button.
- Outcomes / config: `useGraphWorkflowArtifacts` lists + patches on
  `artifact_added`. Theater scopes them in the act inspector with the focused
  node; reveal expands the rail and focuses that act when already on stage.
  Overview shows a per-node count affordance only.

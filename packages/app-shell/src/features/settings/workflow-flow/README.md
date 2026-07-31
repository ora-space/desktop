# workflow-flow

React Flow–based canvas for the settings workflow builder.

## Responsibilities

- Render and edit a workflow graph (`nodes` + `edges`) with pan, zoom, fit-to-view, connect, reconnect, and delete.
- Keep pointer-frequency node/edge changes inside React Flow and commit stable graph mutations to the domain owner.
- Reuse unchanged React Flow element objects when domain state changes so edits only rerender affected cards and connections.
- Provide grid alignment and an interactive minimap when the canvas is wide enough.
- Host the bottom node catalog overlay and panel expand controls.
- Map domain workflow types from `@ora/workflow-mock` to React Flow elements without owning persistence.

## Non-responsibilities

- Does not load, save, or version workflows (owned by `WorkflowSettings` + repository).
- Does not own the left library manager or right inspector.
- Does not own OpenSpec composer stepper state (`workflow-store`).

## Public boundary

- `WorkflowCanvas` — drop-in graph editor used by `WorkflowSettings`.
- Domain positions remain top-left card coordinates; React Flow is an implementation detail of this module.

## Key invariants

- Reject self-loops and duplicate directed `(source, target)` edges at connect/reconnect time.
- Treat the full card as a valid drop zone while retaining directional left/right ports and candidate feedback.
- Render direction with React Flow's native closed-arrow marker rather than separate edge artwork.
- Keep selected-edge reconnect hit areas centered on the visible endpoints; unselected edges never intercept them.
- Node dragging is local and responsive; the final snapped position is committed once when dragging stops.
- Viewport (pan/zoom) is session-local and not persisted in workflow JSON.
- Catalog drop only commits when the pointer is released inside the canvas bounds.

## Interactions

- Parent supplies graph data, backend-provided capabilities, and mutation callbacks (`onMoveNode`, `onConnect`, `onAddNode`, …).
- `useWorkflowFlowState` is the synchronization boundary between React Flow's transient state and domain state.
- `WorkflowNodeCatalog` remains nested here for drop-coordinate conversion via `screenToFlowPosition`.

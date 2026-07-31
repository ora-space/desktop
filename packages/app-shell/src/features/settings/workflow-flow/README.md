# workflow-flow

React Flow–based canvas for the settings workflow builder.

## Responsibilities

- Render and edit a workflow graph (`nodes` + `edges`) with pan, zoom, connect, reconnect, and delete.
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
- Viewport (pan/zoom) is session-local and not persisted in workflow JSON.
- Catalog drop only commits when the pointer is released inside the canvas bounds.

## Interactions

- Parent supplies graph data and mutation callbacks (`onMoveNode`, `onConnect`, `onAddNode`, …).
- `WorkflowNodeCatalog` remains nested here for drop-coordinate conversion via `screenToFlowPosition`.

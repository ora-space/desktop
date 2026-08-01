# workflow-flow

React Flow–based canvas for the session-only settings workflow demo.

## Responsibilities

- Render and edit a controlled workflow graph with pan, zoom, fit-to-view,
  connect, reconnect, selection, and delete interactions.
- Forward React Flow changes directly to the session graph instead of mirroring
  nodes and edges in a second hook-owned store.
- Provide grid alignment, an interactive minimap, the node catalog overlay, and
  panel expand controls.
- Render native React Flow `Node<TData>` and `Edge` elements without adapters.
- Use React Flow's `BaseEdge`, path helpers, selection, deletion, and viewport
  helpers instead of maintaining parallel interaction utilities.

## Non-responsibilities

- Does not load, save, version, or otherwise persist workflows.
- Does not own the left library manager, right inspector, or mock run preview.
- Does not own OpenSpec composer stepper state (`workflow-store`).

## Public boundary

- `WorkflowCanvas` is the graph editor used by `WorkflowSettings`.
- Positions use React Flow's `XYPosition` and remain top-left card coordinates.

## Key invariants

- React Flow nodes and edges are the single source of truth for the graph.
- Self-loops and duplicate directed `(source, target)` edges are rejected.
- The required Start node uses React Flow's `deletable: false`, and the catalog
  does not offer a second Start node.
- The full card is a forgiving connection drop zone while directional ports and
  candidate feedback remain visible.
- Selected-edge reconnect hit areas remain centered on visible endpoints.
- Each session workflow carries React Flow's `ReactFlowJsonObject` viewport so
  switching or importing a workflow restores its exact pan and zoom state.
- Catalog drops only commit inside canvas bounds and snap to the visible grid.

## Interactions

- The parent applies React Flow `NodeChange` and `EdgeChange` events directly
  with `applyNodeChanges` and `applyEdgeChanges`.
- React Flow owns selection semantics and performs node/edge deletion through
  `deleteElements`, including removal of incident edges.
- Executable fields such as instruction, model, tool, and condition use React
  Flow's supported `node.data` extension point.
- `WorkflowNodeCatalog` remains nested for drop-coordinate conversion through
  `screenToFlowPosition`.

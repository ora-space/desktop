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
- Each session workflow carries a `ReactFlowJsonObject`; graph transitions
  capture it with `toObject()` so nodes, edges, selection, and viewport restore
  from the same React Flow snapshot.
- Workflow export captures that same live `toObject()` snapshot before handing
  the pretty-printed JSON to the host save flow.
- Catalog drops only commit inside canvas bounds and snap to the visible grid.
- Published mock versions open in a read-only canvas preview; restoring is the
  only interaction that copies a selected graph back into the active draft.

## Interactions

- The parent applies React Flow `NodeChange` and `EdgeChange` events directly
  with `applyNodeChanges` and `applyEdgeChanges`.
- Node clicks reopen a collapsed inspector when the clicked node is still
  selected (drag-collapse leaves selection intact, so selection alone cannot
  drive the reopen).
- React Flow owns selection semantics and performs node/edge deletion through
  `deleteElements`, including removal of incident edges.
- Executable fields use React Flow's supported `node.data` extension point.
  Agent nodes store their versioned executor, Role, Skill, MCP, and prompt
  configuration in `agentConfig` rather than in card UI state.
- Editor cards render populated execution fields as read-only parameter summaries;
  the right inspector remains the single surface for changing those fields.
- `WorkflowNodeCatalog` remains nested for drop-coordinate conversion through
  `screenToFlowPosition`.

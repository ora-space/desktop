# workflow-flow

React Flow–based canvas for the workspace workflow editor.

## Responsibilities

- Render and edit a controlled workflow graph with pan, zoom, fit-to-view,
  connect, reconnect, selection, and delete interactions.
- Forward React Flow changes directly to the session graph instead of mirroring
  nodes and edges in a second hook-owned store.
- Provide grid alignment, an interactive minimap, pointer/hand interaction
  modes, editor-only annotations, automatic node organization, the node catalog
  overlay, inspector restore controls, and the bottom-left undo/redo/history
  controls.
- Render native React Flow `Node<TData>` and `Edge` elements without adapters.
- Use React Flow's `BaseEdge`, path helpers, selection, deletion, and viewport
  helpers instead of maintaining parallel interaction utilities.

## Non-responsibilities

- Does not load, save, version, or otherwise persist workflows.
- Does not own the sidebar library list or the right inspector.

## Public boundary

- `WorkflowCanvas` is the graph editor used by `WorkflowEditor`.
- Positions use React Flow's `XYPosition` and remain top-left card coordinates.

## Key invariants

- React Flow nodes and edges are the single source of truth for the graph.
- Annotation nodes are persisted beside executable nodes and are never sent to
  workflow normalization or execution.
- Self-loops and duplicate directed `(source, target)` edges are rejected.
- The required Start node uses React Flow's `deletable: false`, and the catalog
  does not offer a second Start node.
- The full card is a forgiving connection drop zone while directional ports and
  candidate feedback remain visible.
- Selected-edge reconnect hit areas remain centered on visible endpoints.
- Each workflow draft carries a `ReactFlowJsonObject`; graph transitions
  capture it with `toObject()` so nodes, edges, selection, and viewport restore
  from the same React Flow snapshot.
- Workflow export captures that same live `toObject()` snapshot before handing
  the pretty-printed JSON to the host save flow.
- Automatic organization moves executable nodes only; annotations retain their
  authored positions and is recorded as one semantic history step.
- Catalog drops only commit inside canvas bounds and snap to the visible grid.
- Published versions open in a read-only canvas preview; activating a version
  copies that graph into the editable draft.
- A muted caption beside the history icon shows unpublished vs the active
  published version, and switches to a read-only preview hint with a return
  action.
- The current-draft row in version history can publish that draft through the
  same dialog as the header Publish action.
- Change history stores complete authored workflow snapshots, coalesces a drag
  or focused text edit into one step, and excludes selection/measurement state.
  The history panel is newest-to-oldest, highlights the current operation,
  names affected nodes and edge endpoints when available, can jump to any
  retained step, and can clear the session stack.

## Interactions

- The parent applies React Flow `NodeChange` and `EdgeChange` events directly
  with `applyNodeChanges` and `applyEdgeChanges`.
- Pointer mode box-selects from a blank-canvas left drag; hand mode pans from
  that same gesture. In both modes, a drag that starts on a node moves the node.
- An annotation behaves like a regular draggable node in its reading state,
  including the grab cursor. A click enters text editing, while a pointer drag
  continues to move the annotation; blur or Escape returns it to dragging.
- Annotations always remain below executable nodes, including while selected or
  dragged, so they provide context without covering workflow controls.
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

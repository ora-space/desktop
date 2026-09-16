# workflow-editor

Product UI for **authoring and publishing** workflow definitions. Hosted as a
first-class workspace surface (sidebar library + main canvas), not a Settings
category.

## Responsibilities

- Persist and edit the workflow library (create, rename, delete, import, export).
- Render the React Flow canvas and the node inspector for the selected draft.
- Autosave the open draft and publish / preview / activate versions.
- Keep a session-only semantic history for undo, redo, and direct history jumps.
- Show unpublished vs the active published version as muted canvas caption
  beside the history control.

## Non-responsibilities

- Does not choose a run Workspace or create `GraphWorkflowRun` rows.
- Does not own Theater / Overview (`workflow-run`).

## Public boundary

- `WorkflowEditor` fills the workspace main pane while `ui-store.workflowEditorOpen`.
- `WorkflowEditorList` replaces the project tree in the app sidebar for that mode.
- Selection and flush-before-switch actions live in `workflow-editor-store`.
- `useWorkflowLibrary` is also consumed by the workspace create menu to start runs.
- Agent-node MCP choices derive from `useInstalledPlugins` (`kind: "mcp"`) and share plugin-query
  invalidation with Settings. Canonical IDs and enabled flags persist in the graph; availability
  is display metadata. Missing or unavailable bindings stay editable, and discovery failure
  offers retry without claiming that installed plugins disappeared. Installation only populates
  the global catalog; enabled bindings are the node Session's allowlist. Agent-node Skill switches
  instead express mandatory invocation and do not provide node-level Skill isolation.

## Key invariants

- Opening the editor replaces the workspace main pane without changing the
  current project/session/run selection. Closing it reveals that same surface;
  chat and run views remount, so their local UI state resets. Editor open
  state is session-only and is not persisted.
- Ctrl/Cmd+N opens the new-workflow dialog while the editor is open instead of
  starting a chat.
- Switching or leaving a draft flushes pending autosave first so unsaved edits
  are not dropped. A failed flush keeps the editor open and reports the error;
  a successful leave clears the sidebar error.
- The inner library rail is gone: the app sidebar is the only workflow list.
  Newest-created workflows are first; create prepends the row and opens its draft.
- The node catalog advertises only the runtime-backed Start, Agent, Condition, Iteration, and
  Output nodes. Prototype metadata for other node kinds remains available so each kind can be
  exposed when its runtime support is implemented.
- The iteration node renders as an embedded container frame on the same canvas: dragging a node
  into the frame's region zone assigns React Flow `parentId` containment, the frame collapses to
  a compact member-count summary, and its inspector edits the iterator source, collect target,
  error strategy, and the iteration ceiling. Editor-side connection rules reject edges that
  cross a region boundary in a direction the engine cannot honor; the authoritative validation
  stays in the Rust graph parser.
- Collapsing the app sidebar hides the library in place; it does not remount
  the canvas, so in-memory draft edits survive.
- The + beside the library title opens a menu with New workflow (Ctrl/Cmd+N still opens it
  directly) and Import workflow; an empty library offers both actions inline.
- Import runs in one dialog with three steps: a drop zone / file picker, a preview, or a
  failure explanation (invalid JSON with line and column, missing name or graph, unknown
  node kind, or a file larger than 5 MB). Nothing is persisted before confirmation. The
  preview resolves MCP/Skill references against the installed catalogs; missing or
  unavailable plugins warn but never block. Install on a missing MCP or Skill opens the plugin
  marketplace with its identity searched; an MCP with incomplete or unreadable
  configuration opens its configuration editor under Manage plugins, an MCP with an invalid
  declaration opens Manage plugins, and an unusable Skill opens the Skills page. Users may create a draft
  only or publish with an editable version, and the new row is marked for the session.
- Export offers the live draft or any published version, lists each recorded plugin
  reference with its enabled state, and previews the exact file content. Files record
  plugin identities and enabled flags only, never packages, secrets, or plugin
  configuration. A published version is embedded in the default filename so re-import
  proposes it again.
- Undo/redo history is scoped to the mounted draft session. Switching drafts,
  activating a version, or leaving the editor clears it; autosave and published
  version history are independent.

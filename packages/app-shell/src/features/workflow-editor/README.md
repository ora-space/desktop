# workflow-editor

Product UI for **authoring and publishing** workflow definitions. Hosted as a
first-class workspace surface (sidebar library + main canvas), not a Settings
category.

## Responsibilities

- Persist and edit the workflow library (create, rename, delete, import, export).
- Render the React Flow canvas and the node inspector for the selected draft.
- Autosave the open draft and publish / preview / activate versions.
- Show unpublished vs the active published version as muted canvas caption
  beside the history control.

## Non-responsibilities

- Does not choose a run Workspace or create `GraphWorkflowRun` rows.
- Does not own OpenSpec composer stepper state (`workflow-store`).
- Does not own Theater / Overview (`workflow-run`).

## Public boundary

- `WorkflowEditor` fills the workspace main pane while `ui-store.workflowEditorOpen`.
- `WorkflowEditorList` replaces the project tree in the app sidebar for that mode.
- Selection and flush-before-switch actions live in `workflow-editor-store`.
- `useWorkflowLibrary` is also consumed by the workspace create menu to start runs.
- Agent-node MCP attachments read `MCP_CATALOG` from this feature, not Settings.

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
- Collapsing the app sidebar hides the library in place; it does not remount
  the canvas, so in-memory draft edits survive.

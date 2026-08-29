# workflow-node-chrome

Shared visual building blocks for workflow **definition** cards (settings editor)
and **run** cards (overview / theater).

## Responsibilities

- Own kind → icon/tone metadata (`getNodeMetadata`).
- Provide `WorkflowNodeCardShell` so editor and run UIs share layout rhythm
  (icon, title, kind chip, description, footer) without sharing React Flow
  interaction (delete, reconnect, catalog).
- Provide the Agent execution-mode mark used beside node titles across editor and run cards.
- Support full-width read-only detail content below the header so definition
  cards can surface configuration without inheriting the icon-column indent.

## Non-responsibilities

- Not a React Flow node by itself (callers own Handles / selection / drag).
- Does not encode run execution status (run feature overlays status via slots).

## Adaptation rule

When settings node styling changes, update the shell (and density tokens) here
first; run overview / theater should pick up the same chrome with optional
`frameClassName` / slot overrides for execution-specific emphasis.

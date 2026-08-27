# @ora/workflow-mock

Native React Flow fixtures, node-data extensions, validation, and deterministic
demo execution for the settings workflow builder.

The package intentionally has no persistence abstraction. The demo owns its graph
for the lifetime of the mounted UI and resets when it is remounted. A future
product backend should introduce its own contract when real storage semantics are
known instead of constraining the prototype around a speculative repository API.

Workflow graph snapshots extend `ReactFlowJsonObject<Node<TData>, Edge>` so
nodes, connections, and viewport use React Flow's native persistence shape.
Nodes use `@xyflow/react`'s `Node<TData>` directly and connections use `Edge`.
Editor annotations use a separate `annotations` collection so they can share
React Flow geometry and selection behavior without becoming executable steps.
Executable fields (`instruction`, `model`, `tool`, and `condition`) live in the
official `Node.data` extension point. There is no parallel workflow node, edge,
or position DTO and no adapter layer.

Agent nodes use a versioned `agentConfig` object rather than an unstructured
model label. It records the CLI/model pair, Role ID, configured Skills with an
explicit enabled state, and custom prompt. The app supplies the
model catalog from the backend's agent-model endpoint; Role and Skill choices
remain a stable local mock catalog until their backend APIs are available.

The UI captures graphs with React Flow's `toObject()` at commit boundaries.
Workflow metadata is added beside that native snapshot without translating its
nodes, edges, or viewport.

`createMockWorkflowVersions` provides immutable published graph snapshots for
the frontend-only version-history UI. Previewing one never mutates the draft;
the UI copies its graph into the in-memory draft only when the user restores it.

`createMockWorkflowNode` owns demo node-data defaults so UI components only
provide interaction-derived `XYPosition` values and localized display text.
`createMockWorkflowCapabilities` supplies the model, Role, Skill, and tool
choices rendered by the inspector, plus the node-type catalog and configuration-field schema.

Imported definitions are validated before entering session state. React Flow's
`isNode` and `isEdge` guards validate its element boundaries; business
validation additionally enforces unique node, edge, and annotation IDs, valid edge endpoints,
registered workflow edge and handle types, finite positions and viewport values,
unique directed connections, exactly one Start node, and the required node-data
shape.

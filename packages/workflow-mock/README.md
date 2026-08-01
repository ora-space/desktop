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
Executable fields (`instruction`, `model`, `tool`, and `condition`) live in the
official `Node.data` extension point. There is no parallel workflow node, edge,
position, or config DTO and no adapter layer.

The UI captures graphs with React Flow's `toObject()` at commit boundaries.
Workflow metadata is added beside that native snapshot without translating its
nodes, edges, or viewport.

`createMockWorkflowNode` owns demo node-data defaults so UI components only
provide interaction-derived `XYPosition` values and localized display text.
`createMockWorkflowCapabilities` supplies the model and tool choices rendered by
the inspector, plus the node-type catalog and configuration-field schema.

Imported definitions are validated before entering session state. React Flow's
`isNode` and `isEdge` guards validate its element boundaries; business
validation additionally enforces unique element IDs, valid edge endpoints,
registered workflow edge and handle types, finite positions and viewport values,
unique directed connections, exactly one Start node, and the required node-data
shape.

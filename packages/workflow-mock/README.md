# @ora/workflow-mock

Frontend-only workflow contracts, fixtures, and an in-memory repository for the
settings workflow builder prototype.

`WorkflowRepository` is the replacement boundary for a future backend. UI code
depends on that async shape instead of reading fixtures directly.

`createMockWorkflowNode` owns prototype-only node configuration defaults so UI
components only provide interaction-derived positions and localized display text.
`createMockWorkflowCapabilities` supplies the model and tool choices rendered by
the inspector, plus the node-type catalog rendered by the canvas dock.

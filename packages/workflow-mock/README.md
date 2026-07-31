# @ora/workflow-mock

Frontend-only workflow contracts, fixtures, and an in-memory repository for the
settings workflow builder prototype.

`WorkflowRepository` is the replacement boundary for a future backend. UI code
depends on that async shape instead of reading fixtures directly. Preview runs
receive the current workflow draft so execution cannot silently fall back to an
older persisted revision. Repositories also expose their data-source kind for
environment-specific UI without hardcoded mock labels.

`createMockWorkflowNode` owns prototype-only node configuration defaults so UI
components only provide interaction-derived positions and localized display text.
`createMockWorkflowCapabilities` supplies the model and tool choices rendered by
the inspector, plus the node-type catalog and configuration-field schema rendered
by the canvas dock and inspector. Applications may inject repository and
capability implementations at the `WorkflowSettings` composition boundary.

Imported and saved definitions are validated before entering repository state.
Validation enforces unique element IDs, valid edge endpoints, finite positions,
unique directed connections, exactly one Start node, and node-kind-specific
configuration.

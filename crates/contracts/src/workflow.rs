use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Public workflow summary without persistence audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct Workflow {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub published_snapshot_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Public snapshot payload including the full graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct WorkflowSnapshot {
    pub id: String,
    pub workflow_id: String,
    pub version: String,
    pub graph: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
}

/// Lightweight workflow summary for list views — no graph data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct WorkflowSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub published_version: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Version metadata without graph content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct WorkflowVersion {
    pub id: String,
    pub version: String,
    pub created_at: i64,
}

// ── Create ──

/// Carries the fields required to create a workflow with an optional initial graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct CreateWorkflowRequest {
    pub name: String,
    pub graph: Option<String>,
}

/// Returns the created workflow and its initial draft snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct CreateWorkflowResponse {
    pub workflow: Workflow,
    pub draft: WorkflowSnapshot,
}

// ── Get by ID ──

/// Identifies the workflow to retrieve by its stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetWorkflowRequest {
    pub workflow_id: String,
}

/// Returns the full workflow detail including draft and published snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetWorkflowResponse {
    pub workflow: Workflow,
    pub draft: WorkflowSnapshot,
    pub published: Option<WorkflowSnapshot>,
}

// ── List ──

/// Requests every visible workflow, newest created first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct ListWorkflowsRequest {}

/// Returns every visible workflow summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct ListWorkflowsResponse {
    pub workflows: Vec<WorkflowSummary>,
}

// ── Update name ──

/// Replaces the name of an existing workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct UpdateWorkflowRequest {
    pub workflow_id: String,
    pub name: String,
}

/// Returns the updated workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct UpdateWorkflowResponse {
    pub workflow: Workflow,
}

// ── Delete workflow ──

/// Identifies the workflow to soft-delete by its stable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct DeleteWorkflowRequest {
    pub workflow_id: String,
}

/// Returns the identifier of the workflow that was soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct DeleteWorkflowResponse {
    pub workflow_id: String,
}

// ── Get draft ──

/// Identifies the workflow whose draft snapshot to retrieve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetDraftRequest {
    pub workflow_id: String,
}

/// Returns the draft snapshot including its full graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetDraftResponse {
    pub snapshot: WorkflowSnapshot,
}

// ── Update draft ──

/// Replaces the graph of the draft snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct UpdateDraftRequest {
    pub workflow_id: String,
    pub graph: String,
}

/// Returns the updated draft snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct UpdateDraftResponse {
    pub snapshot: WorkflowSnapshot,
}

// ── Publish ──

/// Publishes the current draft as an immutable snapshot, with an optional user-provided version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct PublishWorkflowRequest {
    pub workflow_id: String,
    pub version: Option<String>,
}

/// Returns the newly created published snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct PublishWorkflowResponse {
    pub snapshot: WorkflowSnapshot,
}

// ── Rollback ──

/// Copies the graph from a historical snapshot into the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct RollbackWorkflowRequest {
    pub workflow_id: String,
    pub snapshot_id: String,
}

/// Returns the updated draft snapshot after rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct RollbackWorkflowResponse {
    pub snapshot: WorkflowSnapshot,
}

// ── Activate ──

/// Switches the published version pointer and syncs the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct ActivateWorkflowRequest {
    pub workflow_id: String,
    pub snapshot_id: String,
}

/// Returns the updated draft snapshot after activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct ActivateWorkflowResponse {
    pub snapshot: WorkflowSnapshot,
}

// ── List versions ──

/// Identifies the workflow whose version history to retrieve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct ListVersionsRequest {
    pub workflow_id: String,
}

/// Returns the list of published (non-draft, non-deleted) version summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct ListVersionsResponse {
    pub versions: Vec<WorkflowVersion>,
}

// ── Get version ──

/// Identifies the workflow and version to retrieve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetVersionRequest {
    pub workflow_id: String,
    pub version: String,
}

/// Returns the snapshot for the specified version including its full graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetVersionResponse {
    pub snapshot: WorkflowSnapshot,
}

// ── Delete snapshot ──

/// Identifies the workflow and version to soft-delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct DeleteSnapshotRequest {
    pub workflow_id: String,
    pub version: String,
}

/// Returns the identifiers of the soft-deleted snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct DeleteSnapshotResponse {
    pub snapshot_id: String,
    pub version: String,
}

// ── Get snapshot by id ──

/// Identifies a snapshot by its stable identifier, independent of its workflow or version key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetWorkflowSnapshotRequest {
    pub snapshot_id: String,
}

/// Returns the snapshot including its full frozen graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workflow.ts")]
pub struct GetWorkflowSnapshotResponse {
    pub snapshot: WorkflowSnapshot,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    Workflow::export(config)?;
    WorkflowSnapshot::export(config)?;
    WorkflowSummary::export(config)?;
    WorkflowVersion::export(config)?;
    CreateWorkflowRequest::export(config)?;
    CreateWorkflowResponse::export(config)?;
    GetWorkflowRequest::export(config)?;
    GetWorkflowResponse::export(config)?;
    ListWorkflowsRequest::export(config)?;
    ListWorkflowsResponse::export(config)?;
    UpdateWorkflowRequest::export(config)?;
    UpdateWorkflowResponse::export(config)?;
    DeleteWorkflowRequest::export(config)?;
    DeleteWorkflowResponse::export(config)?;
    GetDraftRequest::export(config)?;
    GetDraftResponse::export(config)?;
    UpdateDraftRequest::export(config)?;
    UpdateDraftResponse::export(config)?;
    PublishWorkflowRequest::export(config)?;
    PublishWorkflowResponse::export(config)?;
    RollbackWorkflowRequest::export(config)?;
    RollbackWorkflowResponse::export(config)?;
    ActivateWorkflowRequest::export(config)?;
    ActivateWorkflowResponse::export(config)?;
    ListVersionsRequest::export(config)?;
    ListVersionsResponse::export(config)?;
    GetVersionRequest::export(config)?;
    GetVersionResponse::export(config)?;
    DeleteSnapshotRequest::export(config)?;
    DeleteSnapshotResponse::export(config)?;
    GetWorkflowSnapshotRequest::export(config)?;
    GetWorkflowSnapshotResponse::export(config)?;
    Ok(())
}

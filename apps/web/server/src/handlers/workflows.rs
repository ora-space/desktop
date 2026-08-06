use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    ActivateWorkflowRequest, ActivateWorkflowResponse, CreateWorkflowRequest,
    CreateWorkflowResponse, DeleteSnapshotRequest, DeleteSnapshotResponse, DeleteWorkflowRequest,
    DeleteWorkflowResponse, GetDraftRequest, GetDraftResponse, GetVersionRequest,
    GetVersionResponse, GetWorkflowRequest, GetWorkflowResponse, ListVersionsRequest,
    ListVersionsResponse, ListWorkflowsRequest, ListWorkflowsResponse, PublishWorkflowRequest,
    PublishWorkflowResponse, RollbackWorkflowRequest, RollbackWorkflowResponse, UpdateDraftRequest,
    UpdateDraftResponse, UpdateWorkflowRequest, UpdateWorkflowResponse,
};
use serde::Deserialize;

/// Carries the workflow identifier used by workflow-scoped routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPath {
    workflow_id: String,
}

/// Carries the workflow identifier and version string used by version-scoped routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionPath {
    workflow_id: String,
    version: String,
}

/// Carries the replacement name used by rename routes before the path identifier is attached.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkflowBody {
    name: String,
}

/// Carries the draft graph replacement used by draft-save routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDraftBody {
    graph: String,
}

/// Carries an optional user-provided version for publish routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishWorkflowBody {
    version: Option<String>,
}

/// Carries the snapshot identifier used by rollback and activate routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotTargetBody {
    snapshot_id: String,
}

/// Creates one workflow from its JSON payload.
pub async fn create_workflow(
    State(app_state): State<AppState>,
    Json(request): Json<CreateWorkflowRequest>,
) -> Result<Json<CreateWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .create_workflow(request)
        .map(Json)
        .map_err(Into::into)
}

/// Loads one workflow with its draft and currently published snapshot.
pub async fn get_workflow(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
) -> Result<Json<GetWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .get_workflow(GetWorkflowRequest {
            workflow_id: path.workflow_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Lists every visible workflow.
pub async fn list_workflows(
    State(app_state): State<AppState>,
) -> Result<Json<ListWorkflowsResponse>, WebApiError> {
    app_state
        .backend()
        .list_workflows(ListWorkflowsRequest {})
        .map(Json)
        .map_err(Into::into)
}

/// Replaces one workflow name while preserving its identifier and creation timestamp.
pub async fn update_workflow(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
    Json(body): Json<UpdateWorkflowBody>,
) -> Result<Json<UpdateWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .update_workflow(UpdateWorkflowRequest {
            workflow_id: path.workflow_id,
            name: body.name,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Soft-deletes one workflow and cascades to all its snapshots.
pub async fn delete_workflow(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
) -> Result<Json<DeleteWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .delete_workflow(DeleteWorkflowRequest {
            workflow_id: path.workflow_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Loads one workflow's draft snapshot including its full graph.
pub async fn get_draft(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
) -> Result<Json<GetDraftResponse>, WebApiError> {
    app_state
        .backend()
        .get_workflow_draft(GetDraftRequest {
            workflow_id: path.workflow_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Replaces one workflow's draft graph in place.
pub async fn update_draft(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
    Json(body): Json<UpdateDraftBody>,
) -> Result<Json<UpdateDraftResponse>, WebApiError> {
    app_state
        .backend()
        .update_workflow_draft(UpdateDraftRequest {
            workflow_id: path.workflow_id,
            graph: body.graph,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Publishes one workflow's draft as an immutable versioned snapshot.
pub async fn publish_workflow(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
    Json(body): Json<PublishWorkflowBody>,
) -> Result<Json<PublishWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .publish_workflow(PublishWorkflowRequest {
            workflow_id: path.workflow_id,
            version: body.version,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Copies one historical snapshot's graph back into the draft.
pub async fn rollback_workflow(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
    Json(body): Json<SnapshotTargetBody>,
) -> Result<Json<RollbackWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .rollback_workflow(RollbackWorkflowRequest {
            workflow_id: path.workflow_id,
            snapshot_id: body.snapshot_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Switches the published version pointer and syncs its graph into the draft.
pub async fn activate_workflow(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
    Json(body): Json<SnapshotTargetBody>,
) -> Result<Json<ActivateWorkflowResponse>, WebApiError> {
    app_state
        .backend()
        .activate_workflow(ActivateWorkflowRequest {
            workflow_id: path.workflow_id,
            snapshot_id: body.snapshot_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Lists the published (non-draft, non-deleted) version summaries of one workflow.
pub async fn list_versions(
    State(app_state): State<AppState>,
    Path(path): Path<WorkflowPath>,
) -> Result<Json<ListVersionsResponse>, WebApiError> {
    app_state
        .backend()
        .list_workflow_versions(ListVersionsRequest {
            workflow_id: path.workflow_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Loads one snapshot (draft or published) by version string.
pub async fn get_version(
    State(app_state): State<AppState>,
    Path(path): Path<VersionPath>,
) -> Result<Json<GetVersionResponse>, WebApiError> {
    app_state
        .backend()
        .get_workflow_version(GetVersionRequest {
            workflow_id: path.workflow_id,
            version: path.version,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Soft-deletes one published snapshot subject to the workflow invariants.
pub async fn delete_snapshot(
    State(app_state): State<AppState>,
    Path(path): Path<VersionPath>,
) -> Result<Json<DeleteSnapshotResponse>, WebApiError> {
    app_state
        .backend()
        .delete_workflow_snapshot(DeleteSnapshotRequest {
            workflow_id: path.workflow_id,
            version: path.version,
        })
        .map(Json)
        .map_err(Into::into)
}

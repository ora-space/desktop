//! Desktop workflow operations.

use ora_contracts::*;
use serde::Deserialize;
use std::path::PathBuf;

/// Carries a user-selected destination and serialized workflow definition for export.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteWorkflowExportRequest {
    path: PathBuf,
    content: String,
}

/// Writes a workflow export after the desktop save dialog has selected its exact destination.
#[tauri::command]
pub async fn write_workflow_export(request: WriteWorkflowExportRequest) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || std::fs::write(request.path, request.content))
        .await
        .map_err(|error| format!("workflow export task failed: {error}"))?
        .map_err(|error| format!("workflow export write failed: {error}"))
}

backend_command!(
    create_workflow,
    CreateWorkflowRequest,
    CreateWorkflowResponse,
    workflows.create,
    "Creates one workflow through the shared Backend."
);
backend_command!(
    get_workflow,
    GetWorkflowRequest,
    GetWorkflowResponse,
    workflows.get,
    "Gets one workflow through the shared Backend."
);
backend_command!(
    list_workflows,
    ListWorkflowsRequest,
    ListWorkflowsResponse,
    workflows.list,
    "Lists workflows through the shared Backend."
);
backend_command!(
    update_workflow,
    UpdateWorkflowRequest,
    UpdateWorkflowResponse,
    workflows.update,
    "Updates one workflow through the shared Backend."
);
backend_command!(
    delete_workflow,
    DeleteWorkflowRequest,
    DeleteWorkflowResponse,
    workflows.delete,
    "Deletes one workflow through the shared Backend."
);
backend_command!(
    get_workflow_draft,
    GetDraftRequest,
    GetDraftResponse,
    workflows.get_draft,
    "Gets one workflow's draft snapshot through the shared Backend."
);
backend_command!(
    update_workflow_draft,
    UpdateDraftRequest,
    UpdateDraftResponse,
    workflows.update_draft,
    "Updates one workflow's draft graph through the shared Backend."
);
backend_command!(
    publish_workflow,
    PublishWorkflowRequest,
    PublishWorkflowResponse,
    workflows.publish,
    "Publishes one workflow draft through the shared Backend."
);
backend_command!(
    rollback_workflow,
    RollbackWorkflowRequest,
    RollbackWorkflowResponse,
    workflows.rollback,
    "Rolls back one workflow draft through the shared Backend."
);
backend_command!(
    activate_workflow,
    ActivateWorkflowRequest,
    ActivateWorkflowResponse,
    workflows.activate,
    "Activates one workflow version through the shared Backend."
);
backend_command!(
    list_workflow_versions,
    ListVersionsRequest,
    ListVersionsResponse,
    workflows.list_versions,
    "Lists one workflow's published versions through the shared Backend."
);
backend_command!(
    get_workflow_version,
    GetVersionRequest,
    GetVersionResponse,
    workflows.get_version,
    "Gets one workflow version snapshot through the shared Backend."
);
backend_command!(
    delete_workflow_snapshot,
    DeleteSnapshotRequest,
    DeleteSnapshotResponse,
    workflows.delete_snapshot,
    "Deletes one workflow snapshot through the shared Backend."
);
backend_command!(
    get_workflow_snapshot,
    GetWorkflowSnapshotRequest,
    GetWorkflowSnapshotResponse,
    workflows.get_snapshot,
    "Gets one snapshot by id through the shared Backend."
);

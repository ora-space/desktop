use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{GetWorkflowSnapshotRequest, GetWorkflowSnapshotResponse};
use serde::Deserialize;

/// Carries the snapshot identifier used by snapshot-by-id routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPath {
    snapshot_id: String,
}

/// Loads one snapshot by its stable identifier, independent of its workflow.
pub async fn get_workflow_snapshot(
    State(app_state): State<AppState>,
    Path(path): Path<SnapshotPath>,
) -> Result<Json<GetWorkflowSnapshotResponse>, WebApiError> {
    app_state
        .backend()
        .get_workflow_snapshot(GetWorkflowSnapshotRequest {
            snapshot_id: path.snapshot_id,
        })
        .map(Json)
        .map_err(Into::into)
}

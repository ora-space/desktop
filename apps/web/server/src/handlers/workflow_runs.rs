use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, Query, State};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsRequest, ListWorkflowRunsResponse,
};
use serde::Deserialize;

/// Carries the run identifier used by run-scoped routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPath {
    run_id: String,
}

/// Carries the workflow-run collection scope: exactly one of `projectId` or `workflowId`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRunsQuery {
    project_id: Option<String>,
    workflow_id: Option<String>,
}

/// Creates one pending run against a published snapshot with a dedicated worktree.
pub async fn create_workflow_run(
    State(app_state): State<AppState>,
    Json(request): Json<CreateWorkflowRunRequest>,
) -> Result<Json<CreateWorkflowRunResponse>, WebApiError> {
    app_state
        .backend()
        .create_workflow_run(request)
        .map(Json)
        .map_err(Into::into)
}

/// Loads one run detail including its display name, task id, and node runs.
pub async fn get_workflow_run(
    State(app_state): State<AppState>,
    Path(path): Path<RunPath>,
) -> Result<Json<GetWorkflowRunResponse>, WebApiError> {
    app_state
        .backend()
        .get_workflow_run(GetWorkflowRunRequest {
            run_id: path.run_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Lists runs for either a project or a workflow, dispatching on the supplied query scope.
pub async fn list_workflow_runs(
    State(app_state): State<AppState>,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<ListWorkflowRunsResponse>, WebApiError> {
    if let Some(project_id) = query.project_id {
        return app_state
            .backend()
            .list_workflow_runs(ListWorkflowRunsRequest { project_id })
            .map(Json)
            .map_err(Into::into);
    }
    if let Some(workflow_id) = query.workflow_id {
        let response = app_state
            .backend()
            .list_workflow_runs_by_workflow(ListWorkflowRunsByWorkflowRequest { workflow_id })?;
        return Ok(Json(ListWorkflowRunsResponse {
            runs: response.runs,
        }));
    }
    Err(WebApiError::bad_request(
        "workflow run listing requires a projectId or workflowId query parameter",
    ))
}

/// Lists the node-run history of one run in stable ascending order.
pub async fn list_workflow_node_runs(
    State(app_state): State<AppState>,
    Path(path): Path<RunPath>,
) -> Result<Json<ListWorkflowNodeRunsResponse>, WebApiError> {
    app_state
        .backend()
        .list_workflow_node_runs(ListWorkflowNodeRunsRequest {
            run_id: path.run_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Soft-deletes one non-active run and removes its physical worktree.
pub async fn delete_workflow_run(
    State(app_state): State<AppState>,
    Path(path): Path<RunPath>,
) -> Result<Json<DeleteWorkflowRunResponse>, WebApiError> {
    app_state
        .backend()
        .delete_workflow_run(DeleteWorkflowRunRequest {
            run_id: path.run_id,
        })
        .map(Json)
        .map_err(Into::into)
}

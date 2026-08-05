use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, GetTaskWorkspaceRequest, GetTaskWorkspaceResponse, ListTasksRequest,
    ListTasksResponse, TaskStatus, UpdateTaskRequest, UpdateTaskResponse,
};
use serde::Deserialize;

/// Carries the request path segment used by task identifier routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPath {
    task_id: String,
}

/// Carries the HTTP body used for task update routes before the path identifier is applied.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskBody {
    title: String,
    status: TaskStatus,
}

/// Creates one task by forwarding the request body into the application layer.
pub async fn create_task(
    State(app_state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, WebApiError> {
    app_state
        .backend()
        .create_task(request)
        .map(Json)
        .map_err(WebApiError::from)
}

/// Loads one task by combining the path identifier into the contract request.
pub async fn get_task(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
) -> Result<Json<GetTaskResponse>, WebApiError> {
    app_state
        .backend()
        .get_task(GetTaskRequest {
            task_id: path.task_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Returns the authoritative task workspace used by sessions and directory selection.
pub async fn get_task_workspace(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
) -> Result<Json<GetTaskWorkspaceResponse>, WebApiError> {
    app_state
        .backend()
        .get_task_workspace(GetTaskWorkspaceRequest {
            task_id: path.task_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists every visible task by delegating to the application handler.
pub async fn list_tasks(
    State(app_state): State<AppState>,
) -> Result<Json<ListTasksResponse>, WebApiError> {
    app_state
        .backend()
        .list_tasks(ListTasksRequest {})
        .map(Json)
        .map_err(WebApiError::from)
}

/// Replaces one task by combining the route identifier with the JSON body payload.
pub async fn update_task(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<Json<UpdateTaskResponse>, WebApiError> {
    app_state
        .backend()
        .update_task(UpdateTaskRequest {
            task_id: path.task_id,
            title: body.title,
            status: body.status,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Deletes one task by combining the path identifier into the contract request.
pub async fn delete_task(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
) -> Result<Json<DeleteTaskResponse>, WebApiError> {
    app_state
        .backend()
        .delete_task(DeleteTaskRequest {
            task_id: path.task_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectBranchesRequest, ListProjectBranchesResponse,
    ListProjectsRequest, ListProjectsResponse, UpdateProjectRequest, UpdateProjectResponse,
};
use serde::Deserialize;

/// Carries the request path segment used by project identifier routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPath {
    project_id: String,
}

/// Carries the HTTP body used for project update routes before the path identifier is applied.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectBody {
    name: String,
}

/// Creates one project by forwarding the request body into the application layer.
pub async fn create_project(
    State(app_state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<Json<CreateProjectResponse>, WebApiError> {
    app_state
        .backend()
        .create_project(request)
        .map(Json)
        .map_err(WebApiError::from)
}

/// Loads one project by combining the path identifier into the contract request.
pub async fn get_project(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
) -> Result<Json<GetProjectResponse>, WebApiError> {
    app_state
        .backend()
        .get_project(GetProjectRequest {
            project_id: path.project_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists every visible project by delegating to the application handler.
pub async fn list_projects(
    State(app_state): State<AppState>,
) -> Result<Json<ListProjectsResponse>, WebApiError> {
    app_state
        .backend()
        .list_projects(ListProjectsRequest {})
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists refreshed local and remote refs available as bases for task worktree creation.
pub async fn list_project_branches(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
) -> Result<Json<ListProjectBranchesResponse>, WebApiError> {
    app_state
        .backend()
        .list_project_branches(ListProjectBranchesRequest {
            project_id: path.project_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Replaces one project by combining the route identifier with the JSON body payload.
pub async fn update_project(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
    Json(body): Json<UpdateProjectBody>,
) -> Result<Json<UpdateProjectResponse>, WebApiError> {
    app_state
        .backend()
        .update_project(UpdateProjectRequest {
            project_id: path.project_id,
            name: body.name,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Deletes one project by combining the path identifier into the contract request.
pub async fn delete_project(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
) -> Result<Json<DeleteProjectResponse>, WebApiError> {
    app_state
        .backend()
        .delete_project(DeleteProjectRequest {
            project_id: path.project_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

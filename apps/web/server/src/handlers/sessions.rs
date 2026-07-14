use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    SessionStatus, UpdateSessionRequest, UpdateSessionResponse,
};
use serde::Deserialize;

/// Carries the request path segment used by session identifier routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPath {
    session_id: String,
}

/// Carries the HTTP body used for session update routes before the path identifier is applied.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionBody {
    task_id: String,
    agent_id: String,
    agent_session_id: Option<String>,
    status: SessionStatus,
}

/// Creates one session by forwarding the request body into the application layer.
pub async fn create_session(
    State(app_state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .create_session(request)
        .map(Json)
        .map_err(WebApiError::from)
}

/// Loads one session by combining the path identifier into the contract request.
pub async fn get_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<GetSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .get_session(GetSessionRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists every visible session by delegating to the application handler.
pub async fn list_sessions(
    State(app_state): State<AppState>,
) -> Result<Json<ListSessionsResponse>, WebApiError> {
    app_state
        .session_api()
        .list_sessions(ListSessionsRequest {})
        .map(Json)
        .map_err(WebApiError::from)
}

/// Replaces one session by combining the route identifier with the JSON body payload.
pub async fn update_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<UpdateSessionBody>,
) -> Result<Json<UpdateSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .update_session(UpdateSessionRequest {
            session_id: path.session_id,
            task_id: body.task_id,
            agent_id: body.agent_id,
            agent_session_id: body.agent_session_id,
            status: body.status,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Deletes one session by combining the path identifier into the contract request.
pub async fn delete_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<DeleteSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .delete_session(DeleteSessionRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

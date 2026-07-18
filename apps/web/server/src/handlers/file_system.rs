use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use ora_contracts::{ListDirectoryRequest, ListDirectoryResponse};
use std::sync::Arc;

/// Lists one server-side directory for the Web platform path picker.
pub async fn list_directory(
    State(app_state): State<AppState>,
    Query(request): Query<ListDirectoryRequest>,
) -> Result<Json<ListDirectoryResponse>, WebApiError> {
    let file_system_api = Arc::clone(app_state.file_system_api());

    // Directory metadata can block on large or network-mounted filesystems, so keep it off Tokio's
    // async worker threads even though the public handler remains asynchronous.
    tokio::task::spawn_blocking(move || file_system_api.list_directory(request))
        .await
        .map_err(|_| {
            WebApiError::file_system(
                StatusCode::INTERNAL_SERVER_ERROR,
                "file_system_worker_failed",
                "filesystem directory worker failed",
            )
        })?
        .map(Json::from)
        .map_err(WebApiError::from)
}

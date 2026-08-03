use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::State;
use ora_contracts::{GetGitIdentityRequest, GitIdentityResponse};

/// Returns the host's global Git identity for the sidebar user profile.
pub async fn get_identity(
    State(app_state): State<AppState>,
) -> Result<Json<GitIdentityResponse>, WebApiError> {
    let backend = app_state.backend().clone();

    // Reading global config spawns the Git CLI, so keep it off Tokio's async worker threads.
    tokio::task::spawn_blocking(move || backend.read_git_identity(GetGitIdentityRequest {}))
        .await
        .map_err(|source| WebApiError::internal("git identity worker failed", source))?
        .map(Json::from)
        .map_err(WebApiError::from)
}

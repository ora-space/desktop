//! Desktop git identity operations.

use crate::error::CommandError;
use ora_contracts::{GetGitIdentityRequest, GitIdentityResponse};

/// Reads the host identity without retaining the application's storage or runtime composition.
#[tauri::command]
pub async fn get_git_identity(
    request: GetGitIdentityRequest,
) -> Result<GitIdentityResponse, CommandError> {
    super::run_backend("get_git_identity", (), request, |_context, _request| {
        Ok(ora_backend::resolve_git_identity())
    })
    .await
}

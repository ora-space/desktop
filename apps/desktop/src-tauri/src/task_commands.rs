use crate::error::CommandError;
use crate::state::DesktopState;
use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::*;
use tauri::State;

/// Returns the authoritative task root and optional linked-worktree branch.
#[tauri::command]
pub async fn get_task_workspace(
    state: State<'_, DesktopState>,
    request: GetTaskWorkspaceRequest,
) -> Result<GetTaskWorkspaceResponse, CommandError> {
    run_blocking(
        "get_task_workspace",
        state.backend.clone(),
        move |backend| backend.get_task_workspace(request),
    )
    .await
}

/// Runs one synchronous Backend operation without blocking the Tauri async runtime.
async fn run_blocking<Response, Operation>(
    operation_name: &'static str,
    backend: ora_backend::Backend,
    operation: Operation,
) -> Result<Response, CommandError>
where
    Response: Send + 'static,
    Operation: FnOnce(&ora_backend::Backend) -> Result<Response, BackendError> + Send + 'static,
{
    let lifecycle = RequestLifecycle::start(operation_name, &UuidRequestIdGenerator);
    match tauri::async_runtime::spawn_blocking(move || operation(&backend)).await {
        Ok(Ok(response)) => {
            lifecycle.complete_success();
            Ok(response)
        }
        Ok(Err(error)) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        Err(source) => Err(CommandError::from_backend_with_lifecycle(
            BackendError::internal("Desktop task command failed", source),
            &lifecycle,
        )),
    }
}

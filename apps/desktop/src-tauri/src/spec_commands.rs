use crate::commands::{forward_workspace_watch, register_contract_stream};
use crate::error::CommandError;
use crate::state::DesktopState;
use crate::workspace_files::workspace_file_backend_error;
use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::*;
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;

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

/// Returns one effective specification catalog through the shared asynchronous Backend.
#[tauri::command]
pub async fn get_spec_catalog(
    state: State<'_, DesktopState>,
    request: GetSpecCatalogRequest,
) -> Result<SpecCatalogResponse, CommandError> {
    let lifecycle = RequestLifecycle::start("get_spec_catalog", &UuidRequestIdGenerator);
    state
        .backend
        .get_spec_catalog(request)
        .await
        .inspect(|_response| {
            lifecycle.complete_success();
        })
        .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))
}

/// Reads one catalog-authorized Markdown document through the shared Backend.
#[tauri::command]
pub async fn read_spec(
    state: State<'_, DesktopState>,
    request: ReadSpecRequest,
) -> Result<ReadSpecResponse, CommandError> {
    let lifecycle = RequestLifecycle::start("read_spec", &UuidRequestIdGenerator);
    state
        .backend
        .read_spec(request)
        .await
        .inspect(|_response| {
            lifecycle.complete_success();
        })
        .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))
}

/// Validates one platform-selected specification source directory.
#[tauri::command]
pub async fn resolve_spec_source(
    state: State<'_, DesktopState>,
    request: ResolveSpecSourceRequest,
) -> Result<ResolveSpecSourceResponse, CommandError> {
    run_blocking(
        "resolve_spec_source",
        state.backend.clone(),
        move |backend| backend.resolve_spec_source(request),
    )
    .await
}

/// Atomically replaces one project's specification source overrides.
#[tauri::command]
pub async fn update_project_spec_sources(
    state: State<'_, DesktopState>,
    request: UpdateProjectSpecSourcesRequest,
) -> Result<UpdateProjectSpecSourcesResponse, CommandError> {
    run_blocking(
        "update_project_spec_sources",
        state.backend.clone(),
        move |backend| backend.update_project_spec_sources(request),
    )
    .await
}

/// Starts a specification watcher inside the existing exactly-once stream lifecycle.
pub(crate) async fn start_watch(
    state: State<'_, DesktopState>,
    request: serde_json::Value,
    stream_call_id: String,
    on_event: Channel<serde_json::Value>,
    lifecycle: RequestLifecycle,
    cancellation: CancellationToken,
) -> Result<(), CommandError> {
    let request = serde_json::from_value::<WatchSpecsRequest>(request).map_err(|source| {
        CommandError::from_backend_with_lifecycle(
            BackendError::internal("failed to decode spec watch request", source),
            &lifecycle,
        )
    })?;
    let backend = state.backend.clone();
    let root =
        tauri::async_runtime::spawn_blocking(move || backend.resolve_spec_watch_root(&request))
            .await
            .map_err(|source| {
                CommandError::from_backend_with_lifecycle(
                    BackendError::internal("Desktop spec watch root resolution failed", source),
                    &lifecycle,
                )
            })?
            .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
    let workspace_files = Arc::clone(&state.workspace_files);
    let watcher = tauri::async_runtime::spawn_blocking(move || workspace_files.watch(&root))
        .await
        .map_err(|source| {
            CommandError::from_backend_with_lifecycle(
                BackendError::internal("Desktop spec watcher setup failed", source),
                &lifecycle,
            )
        })?
        .map_err(|error| {
            CommandError::from_backend_with_lifecycle(
                workspace_file_backend_error(error),
                &lifecycle,
            )
        })?;
    register_contract_stream(&state, &stream_call_id, &cancellation)
        .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
    let registry = state.stream_cancellations.clone();
    tauri::async_runtime::spawn(forward_workspace_watch(
        watcher,
        cancellation,
        stream_call_id,
        registry,
        on_event,
        lifecycle,
    ));
    Ok(())
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
            BackendError::internal("Desktop spec command failed", source),
            &lifecycle,
        )),
    }
}

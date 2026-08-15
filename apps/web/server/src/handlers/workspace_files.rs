use crate::app_state::AppState;
use crate::error::WebApiError;
use crate::handlers::ndjson_stream::stream_response;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::Response;
use ora_contracts::{
    ListWorkspaceDirectoryResponse, ReadWorkspaceFileResponse, SearchWorkspaceResponse,
    WorkspaceFileChange, WorkspaceFileEventBatch, WorkspaceSearchKind,
};
use ora_fs::{WorkspaceChange, WorkspaceChangeKind};
use serde::Deserialize;
use std::path::{Path as FilePath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

/// Carries the task identifier owned by every workspace-file route.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPath {
    task_id: String,
}

/// Carries an optional relative directory after the task identifier is applied from the URL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryBody {
    path: Option<String>,
}

/// Carries one required workspace-relative file path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileBody {
    path: String,
}

/// Carries one search query and mode after the task identifier is applied from the URL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchBody {
    query: String,
    kind: WorkspaceSearchKind,
}

/// Lists one immediate directory in the task's active managed worktree.
pub async fn list_directory(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
    Json(body): Json<ListDirectoryBody>,
) -> Result<Json<ListWorkspaceDirectoryResponse>, WebApiError> {
    let root = resolve_workspace_root(&app_state, path.task_id).await?;
    let api = Arc::clone(app_state.workspace_file_api());
    tokio::task::spawn_blocking(move || {
        api.list_directory(&root, FilePath::new(body.path.as_deref().unwrap_or("")))
    })
    .await
    .map_err(|source| WebApiError::internal("workspace directory worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Reads one bounded UTF-8 file in the task's active managed worktree.
pub async fn read_file(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
    Json(body): Json<ReadFileBody>,
) -> Result<Json<ReadWorkspaceFileResponse>, WebApiError> {
    let root = resolve_workspace_root(&app_state, path.task_id).await?;
    let api = Arc::clone(app_state.workspace_file_api());
    tokio::task::spawn_blocking(move || api.read_file(&root, FilePath::new(&body.path)))
        .await
        .map_err(|source| WebApiError::internal("workspace file worker failed", source))?
        .map(Json)
        .map_err(WebApiError::from)
}

/// Runs one cancellable HTTP-scoped ripgrep query inside the task workspace.
pub async fn search(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
    Json(body): Json<SearchBody>,
) -> Result<Json<SearchWorkspaceResponse>, WebApiError> {
    let root = resolve_workspace_root(&app_state, path.task_id).await?;
    app_state
        .workspace_file_api()
        .search(&root, &body.query, body.kind)
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Streams debounced native filesystem events until the HTTP consumer disconnects or the server shuts down.
pub async fn watch(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
) -> Result<Response<Body>, WebApiError> {
    let root = resolve_workspace_root(&app_state, path.task_id).await?;
    let api = Arc::clone(app_state.workspace_file_api());
    let watcher = tokio::task::spawn_blocking(move || api.watch(&root))
        .await
        .map_err(|source| WebApiError::internal("workspace watcher worker failed", source))?
        .map_err(WebApiError::from)?;
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    tokio::task::spawn_blocking(move || {
        while !sender.is_closed() {
            match watcher.receive_batch(WATCH_DEBOUNCE) {
                Ok(Some(changes)) if !changes.is_empty() => {
                    if sender
                        .blocking_send(Ok(WorkspaceFileEventBatch {
                            changes: changes.into_iter().map(to_contract_change).collect(),
                        }))
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Some(_)) | Ok(None) => {}
                Err(error) => {
                    let error = WebApiError::from(error).into_backend_error();
                    let _ = sender.blocking_send(Err(error));
                    break;
                }
            }
        }
    });
    Ok(stream_response(receiver, app_state.shutdown_token()))
}

/// Resolves the task-owned worktree once so filesystem calls never accept a root from the client.
async fn resolve_workspace_root(
    app_state: &AppState,
    task_id: String,
) -> Result<PathBuf, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend
            .resolve_task_cwd(&task_id)
            .map_err(WebApiError::from)
    })
    .await
    .map_err(|source| WebApiError::internal("task workspace worker failed", source))?
}

/// Maps crate-native watcher events into the stable transport contract.
pub(crate) fn to_contract_change(change: WorkspaceChange) -> WorkspaceFileChange {
    match change.kind {
        WorkspaceChangeKind::Created => WorkspaceFileChange::Created { path: change.path },
        WorkspaceChangeKind::Modified => WorkspaceFileChange::Modified { path: change.path },
        WorkspaceChangeKind::Removed => WorkspaceFileChange::Removed { path: change.path },
        WorkspaceChangeKind::Renamed { from } => WorkspaceFileChange::Renamed {
            from,
            path: change.path,
        },
        WorkspaceChangeKind::RescanRequired => WorkspaceFileChange::RescanRequired,
    }
}

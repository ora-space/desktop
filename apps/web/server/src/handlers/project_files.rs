use crate::app_state::AppState;
use crate::error::WebApiError;
use crate::handlers::workspace_files::{stream_response, to_contract_change};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::Response;
use ora_contracts::{
    ListProjectDirectoryResponse, ReadProjectFileResponse, SearchProjectResponse,
    WorkspaceFileEventBatch, WorkspaceSearchKind,
};
use serde::Deserialize;
use std::path::{Path as FilePath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

/// Carries the project identifier owned by every project-file route.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPath {
    project_id: String,
}

/// Carries an optional relative directory after the project identifier is applied from the URL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryBody {
    path: Option<String>,
}

/// Carries one required project-relative file path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileBody {
    path: String,
}

/// Carries one project-file search query after the project identifier is applied from the URL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchBody {
    query: String,
    kind: WorkspaceSearchKind,
}

/// Lists one immediate directory in the project's main checkout.
pub async fn list_directory(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
    Json(body): Json<ListDirectoryBody>,
) -> Result<Json<ListProjectDirectoryResponse>, WebApiError> {
    let root = resolve_project_root(&app_state, path.project_id).await?;
    let api = Arc::clone(app_state.workspace_file_api());
    tokio::task::spawn_blocking(move || {
        api.list_project_directory(&root, FilePath::new(body.path.as_deref().unwrap_or("")))
    })
    .await
    .map_err(|source| WebApiError::internal("project directory worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Reads one bounded UTF-8 file in the project's main checkout.
pub async fn read_file(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
    Json(body): Json<ReadFileBody>,
) -> Result<Json<ReadProjectFileResponse>, WebApiError> {
    let root = resolve_project_root(&app_state, path.project_id).await?;
    let api = Arc::clone(app_state.workspace_file_api());
    tokio::task::spawn_blocking(move || api.read_project_file(&root, FilePath::new(&body.path)))
        .await
        .map_err(|source| WebApiError::internal("project file worker failed", source))?
        .map(Json)
        .map_err(WebApiError::from)
}

/// Runs one cancellable HTTP-scoped ripgrep query inside the project's main checkout.
pub async fn search(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
    Json(body): Json<SearchBody>,
) -> Result<Json<SearchProjectResponse>, WebApiError> {
    let root = resolve_project_root(&app_state, path.project_id).await?;
    app_state
        .workspace_file_api()
        .search_project(&root, &body.query, body.kind)
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Streams debounced native filesystem events until the HTTP consumer disconnects.
pub async fn watch(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
) -> Result<Response<Body>, WebApiError> {
    let root = resolve_project_root(&app_state, path.project_id).await?;
    let api = Arc::clone(app_state.workspace_file_api());
    let watcher = tokio::task::spawn_blocking(move || api.watch(&root))
        .await
        .map_err(|source| WebApiError::internal("project watcher worker failed", source))?
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
    Ok(stream_response(receiver))
}

/// Resolves the persisted project checkout without accepting a root path from the client.
async fn resolve_project_root(
    app_state: &AppState,
    project_id: String,
) -> Result<PathBuf, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend
            .resolve_project_root(&project_id)
            .map_err(WebApiError::from)
    })
    .await
    .map_err(|source| WebApiError::internal("project root worker failed", source))?
}

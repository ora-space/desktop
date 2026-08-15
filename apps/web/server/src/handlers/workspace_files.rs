use crate::app_state::AppState;
use crate::error::{DeferredCompletion, WebApiError, current_lifecycle};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Response, header};
use futures_util::stream;
use ora_backend::BackendError;
use ora_contracts::{
    ContractError, EmptyErrorParams, ListWorkspaceDirectoryResponse, PublicError,
    ReadWorkspaceFileResponse, SearchWorkspaceResponse, WorkspaceFileChange,
    WorkspaceFileEventBatch, WorkspaceSearchKind,
};
use ora_fs::{WorkspaceChange, WorkspaceChangeKind};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::{Path as FilePath, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

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

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamFrame<Event> {
    Data { data: Event },
    Error { error: ContractError },
    End,
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

/// Converts watcher batches into the same private NDJSON framing used by session streams.
pub(crate) fn stream_response(
    receiver: tokio::sync::mpsc::Receiver<Result<WorkspaceFileEventBatch, BackendError>>,
    shutdown: CancellationToken,
) -> Response<Body> {
    let lifecycle = current_lifecycle();
    let body_stream = stream::unfold(
        (receiver, false, lifecycle, shutdown),
        |(mut receiver, ended, lifecycle, shutdown)| async move {
            if ended {
                return None;
            }
            let (frame, next_ended) = tokio::select! {
                _ = shutdown.cancelled() => {
                    lifecycle.complete_success();
                    (StreamFrame::End, true)
                }
                event = receiver.recv() => match event {
                    Some(Ok(event)) => (StreamFrame::Data { data: event }, false),
                    Some(Err(error)) => {
                        lifecycle.complete_failure(&error);
                        (
                            StreamFrame::Error {
                                error: error.contract_error(lifecycle.request_id()),
                            },
                            true,
                        )
                    }
                    None => {
                        lifecycle.complete_success();
                        (StreamFrame::End, true)
                    }
                }
            };
            let mut bytes = serde_json::to_vec(&frame).unwrap_or_else(|source| {
                let error = BackendError::internal("failed to encode stream frame", source);
                lifecycle.complete_failure(&error);
                serde_json::to_vec(&StreamFrame::<WorkspaceFileEventBatch>::Error {
                    error: ContractError {
                        error: PublicError::InternalError(EmptyErrorParams {}),
                        request_id: lifecycle.request_id(),
                    },
                })
                .unwrap_or_default()
            });
            bytes.push(b'\n');
            Some((
                Ok::<Bytes, Infallible>(Bytes::from(bytes)),
                (receiver, next_ended, lifecycle, shutdown),
            ))
        },
    );
    let mut response = Response::new(Body::from_stream(body_stream));
    response.extensions_mut().insert(DeferredCompletion);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::stream_response;
    use futures_util::StreamExt;
    use ora_backend::BackendError;
    use ora_contracts::WorkspaceFileEventBatch;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    /// Verifies a live workspace watch still ends when process shutdown is requested.
    #[tokio::test]
    async fn workspace_watch_stream_ends_when_shutdown_is_requested() {
        let (sender, receiver) =
            tokio::sync::mpsc::channel::<Result<WorkspaceFileEventBatch, BackendError>>(1);
        let shutdown = CancellationToken::new();
        let response = stream_response(receiver, shutdown.clone());
        let mut body = response.into_body().into_data_stream();

        assert!(
            tokio::time::timeout(Duration::from_millis(50), body.next())
                .await
                .is_err(),
            "a live watcher must stay open until shutdown or a filesystem event"
        );

        sender
            .send(Ok(WorkspaceFileEventBatch {
                changes: Vec::new(),
            }))
            .await
            .unwrap_or_else(|error| panic!("send watch batch: {error}"));
        let data = next_frame(&mut body).await;
        assert_eq!(
            data,
            json!({
                "type": "data",
                "data": { "changes": [] }
            })
        );

        shutdown.cancel();
        let end = next_frame(&mut body).await;
        assert_eq!(end, json!({ "type": "end" }));
        let finished = tokio::time::timeout(Duration::from_millis(200), body.next()).await;
        assert!(
            matches!(finished, Ok(None)),
            "the body must complete after the end frame, got {finished:?}"
        );
        drop(sender);
    }

    /// Reads one NDJSON transport frame from a watch body.
    async fn next_frame<E>(
        body: &mut (impl StreamExt<Item = Result<axum::body::Bytes, E>> + Unpin),
    ) -> serde_json::Value
    where
        E: std::fmt::Debug,
    {
        let chunk = tokio::time::timeout(Duration::from_secs(1), body.next())
            .await
            .unwrap_or_else(|_| panic!("watch frame timed out"))
            .unwrap_or_else(|| panic!("watch frame is missing"))
            .unwrap_or_else(|error| panic!("watch frame: {error:?}"));
        serde_json::from_slice(chunk.trim_ascii())
            .unwrap_or_else(|error| panic!("watch frame json: {error}"))
    }
}

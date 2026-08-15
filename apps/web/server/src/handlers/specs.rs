use crate::app_state::AppState;
use crate::error::WebApiError;
use crate::handlers::ndjson_stream::stream_response;
use crate::handlers::workspace_files::to_contract_change;
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::Response;
use ora_contracts::{
    GetSpecCatalogRequest, ReadSpecRequest, SpecCatalogResponse, WatchSpecsRequest,
    WorkspaceFileEventBatch,
};
use std::sync::Arc;
use std::time::Duration;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

/// Returns the effective bounded catalog from the shared Backend composition.
pub async fn catalog(
    State(app_state): State<AppState>,
    Json(request): Json<GetSpecCatalogRequest>,
) -> Result<Json<SpecCatalogResponse>, WebApiError> {
    app_state
        .backend()
        .get_spec_catalog(request)
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Reads one catalog-authorized Markdown document.
pub async fn read(
    State(app_state): State<AppState>,
    Json(request): Json<ReadSpecRequest>,
) -> Result<Json<ora_contracts::ReadSpecResponse>, WebApiError> {
    app_state
        .backend()
        .read_spec(request)
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Streams workspace file events until the HTTP consumer disconnects or the server shuts down.
pub async fn watch(
    State(app_state): State<AppState>,
    Json(request): Json<WatchSpecsRequest>,
) -> Result<Response<Body>, WebApiError> {
    let root = app_state
        .backend()
        .resolve_spec_watch_root(&request)
        .map_err(WebApiError::from)?;
    let api = Arc::clone(app_state.workspace_file_api());
    let watcher = tokio::task::spawn_blocking(move || api.watch(&root))
        .await
        .map_err(|source| WebApiError::internal("spec watcher worker failed", source))?
        .map_err(WebApiError::from)?;
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    tokio::task::spawn_blocking(move || {
        while !sender.is_closed() {
            match watcher.receive_batch(WATCH_DEBOUNCE) {
                Ok(Some(changes)) if !changes.is_empty() => {
                    let batch = WorkspaceFileEventBatch {
                        changes: changes.into_iter().map(to_contract_change).collect(),
                    };
                    if sender.blocking_send(Ok(batch)).is_err() {
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

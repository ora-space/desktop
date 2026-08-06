use crate::app_state::AppState;
use crate::error::WebApiError;
use crate::handlers::workspace_files::{stream_response, to_contract_change};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::Response;
use ora_contracts::{
    GetSpecCatalogRequest, ProjectSpecSourceOverride, ReadSpecRequest, ResolveSpecSourceRequest,
    SpecCatalogResponse, UpdateProjectSpecSourcesRequest, UpdateProjectSpecSourcesResponse,
    WatchSpecsRequest, WorkspaceFileEventBatch,
};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPath {
    project_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSourcesBody {
    sources: Vec<ProjectSpecSourceOverride>,
}

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

/// Validates one directory returned by the existing platform path picker.
pub async fn resolve_source(
    State(app_state): State<AppState>,
    Json(request): Json<ResolveSpecSourceRequest>,
) -> Result<Json<ora_contracts::ResolveSpecSourceResponse>, WebApiError> {
    app_state
        .backend()
        .resolve_spec_source(request)
        .map(Json)
        .map_err(WebApiError::from)
}

/// Atomically replaces project-wide source overrides after applying the route-owned project id.
pub async fn update_project_sources(
    State(app_state): State<AppState>,
    Path(path): Path<ProjectPath>,
    Json(body): Json<UpdateSourcesBody>,
) -> Result<Json<UpdateProjectSpecSourcesResponse>, WebApiError> {
    app_state
        .backend()
        .update_project_spec_sources(UpdateProjectSpecSourcesRequest {
            project_id: path.project_id,
            sources: body.sources,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Streams the shared workspace event format for the target root resolved by Backend.
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
    Ok(stream_response(receiver))
}

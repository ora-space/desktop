use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::Path as AxumPath;
use axum::extract::{Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ora_backend::Backend;
use ora_contracts::{
    CancelSkillImportRequest, CancelSkillImportResponse, CommitSkillImportRequest,
    GetSkillImportSessionRequest, GetSkillImportSessionResponse, PrepareSkillImportRequest,
    PrepareSkillImportResponse, SkillImportConflictDecision, SkillImportSource,
};
use ora_skill_package::path::RelativePath;
use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

/// Carries the upload mode chosen by the client before the multipart body is parsed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareSkillImportQuery {
    mode: String,
}

/// Carries the path identifier used to address one import session resource.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillImportSessionPath {
    session_id: String,
}

/// Carries conflict decisions before the route session id is attached.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSkillImportBody {
    decisions: Vec<SkillImportConflictDecision>,
}

/// Cap for one raw archive upload streamed by the web adapter.
const MAX_ARCHIVE_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Cap for one folder upload streamed by the web adapter.
const MAX_FOLDER_UPLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// Receives a folder or archive multipart upload, materializes it, and prepares the session.
pub async fn prepare_skill_import(
    State(app_state): State<AppState>,
    Query(query): Query<PrepareSkillImportQuery>,
    multipart: Multipart,
) -> Result<Json<PrepareSkillImportResponse>, WebApiError> {
    let upload_root =
        std::env::temp_dir().join(format!("ora-skill-import-upload-{}", Uuid::new_v4()));
    fs::create_dir_all(&upload_root)
        .map_err(|_| WebApiError::bad_request("upload_storage_error"))?;

    let source = match query.mode.as_str() {
        "archive" => receive_archive_upload(multipart, &upload_root).await?,
        "folder" => receive_folder_upload(multipart, &upload_root).await?,
        _ => {
            let _ = fs::remove_dir_all(&upload_root);
            return Err(WebApiError::bad_request("import_upload_mode_invalid"));
        }
    };

    let request = PrepareSkillImportRequest { source };
    let response = prepare_with_backend(app_state.backend().clone(), request).await;
    let _ = fs::remove_dir_all(&upload_root);
    response
}

/// Runs the potentially long preparation on a blocking executor away from async workers.
async fn prepare_with_backend(
    backend: Backend,
    request: PrepareSkillImportRequest,
) -> Result<Json<PrepareSkillImportResponse>, WebApiError> {
    tokio::task::spawn_blocking(move || backend.prepare_skill_import(request))
        .await
        .map_err(|_| WebApiError::bad_request("import_preparation_failed"))?
        .map(Json)
        .map_err(Into::into)
}

/// Gets one import session with its current status and progress.
pub async fn get_skill_import(
    State(app_state): State<AppState>,
    AxumPath(path): AxumPath<SkillImportSessionPath>,
) -> Result<Json<GetSkillImportSessionResponse>, WebApiError> {
    app_state
        .backend()
        .get_skill_import(GetSkillImportSessionRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Accepts and freezes one commit, responding with `202 Accepted` for the background task.
pub async fn commit_skill_import(
    State(app_state): State<AppState>,
    AxumPath(path): AxumPath<SkillImportSessionPath>,
    Json(body): Json<CommitSkillImportBody>,
) -> Result<Response, WebApiError> {
    let response = app_state
        .backend()
        .commit_skill_import(CommitSkillImportRequest {
            session_id: path.session_id,
            decisions: body.decisions,
        })
        .map_err(WebApiError::from)?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

/// Cancels a prepared import session.
pub async fn cancel_skill_import(
    State(app_state): State<AppState>,
    AxumPath(path): AxumPath<SkillImportSessionPath>,
) -> Result<Json<CancelSkillImportResponse>, WebApiError> {
    app_state
        .backend()
        .cancel_skill_import(CancelSkillImportRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Streams one archive part into a temp file, enforcing the raw archive size limit.
async fn receive_archive_upload(
    mut multipart: Multipart,
    upload_root: &Path,
) -> Result<SkillImportSource, WebApiError> {
    let part = match multipart.next_field().await.map_err(upload_read_error)? {
        Some(part) => part,
        None => return Err(WebApiError::bad_request("import_upload_empty")),
    };
    let file_name = part
        .file_name()
        .map(ToString::to_string)
        .ok_or_else(|| WebApiError::bad_request("import_upload_missing_filename"))?;

    let destination = upload_root.join("source-upload");
    stream_part_to_file(
        part,
        &destination,
        MAX_ARCHIVE_UPLOAD_BYTES,
        "archive_too_large",
    )
    .await?;
    if multipart
        .next_field()
        .await
        .map_err(upload_read_error)?
        .is_some()
    {
        return Err(WebApiError::bad_request("import_upload_mixed_sources"));
    }
    Ok(SkillImportSource::Archive {
        path: destination.to_string_lossy().into_owned(),
        file_name,
    })
}

/// Streams folder parts into the upload root under validated relative paths.
async fn receive_folder_upload(
    mut multipart: Multipart,
    upload_root: &Path,
) -> Result<SkillImportSource, WebApiError> {
    let mut total_bytes = 0u64;
    let mut received_any = false;
    while let Some(part) = multipart.next_field().await.map_err(upload_read_error)? {
        received_any = true;
        let relative = part
            .file_name()
            .map(ToString::to_string)
            .ok_or_else(|| WebApiError::bad_request("import_upload_missing_filename"))?;
        let relative = RelativePath::parse(&relative)
            .map_err(|_| WebApiError::bad_request("import_upload_unsafe_path"))?;
        let destination = relative.to_path(upload_root);
        let parent = destination
            .parent()
            .ok_or_else(|| WebApiError::bad_request("import_upload_unsafe_path"))?;
        fs::create_dir_all(parent).map_err(|_| WebApiError::bad_request("upload_storage_error"))?;
        let written = stream_part_to_file(
            part,
            &destination,
            MAX_FOLDER_UPLOAD_BYTES - total_bytes,
            "archive_total_bytes_exceeded",
        )
        .await?;
        total_bytes += written;
    }
    if !received_any {
        return Err(WebApiError::bad_request("import_upload_empty"));
    }
    Ok(SkillImportSource::Folder {
        path: upload_root.to_string_lossy().into_owned(),
    })
}

/// Streams one multipart field into a file, aborting once the byte budget is exceeded.
async fn stream_part_to_file(
    mut part: axum::extract::multipart::Field<'_>,
    destination: &Path,
    byte_budget: u64,
    error_code: &'static str,
) -> Result<u64, WebApiError> {
    let mut output = fs::File::create(destination)
        .map_err(|_| WebApiError::bad_request("upload_storage_error"))?;
    let mut written = 0u64;
    while let Some(chunk) = part.chunk().await.map_err(upload_read_error)? {
        written += chunk.len() as u64;
        if written > byte_budget {
            return Err(WebApiError::bad_request(error_code));
        }
        output
            .write_all(&chunk)
            .map_err(|_| WebApiError::bad_request("upload_storage_error"))?;
    }
    Ok(written)
}

/// Maps a multipart stream failure into a stable upload error.
fn upload_read_error(_: axum::extract::multipart::MultipartError) -> WebApiError {
    WebApiError::bad_request("import_upload_read_failed")
}

use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Multipart, Path, State, multipart::MultipartError};
use axum::http::StatusCode;
use ora_application::UploadedSkillFile;
use ora_backend::ErrorClassification;
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    EmptyErrorParams, GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse,
    PublicError, SkillUploadTooLargeParams, UpdateSkillRequest, UpdateSkillResponse,
};
use serde::Deserialize;

/// Caps one skill folder upload before its multipart fields are materialized in memory.
pub const MAX_SKILL_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

/// Carries the path identifier used to address one skill resource.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPath {
    skill_id: String,
}

/// Carries a replacement payload before the path identifier is attached.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkillBody {
    name: String,
    description: String,
}

/// Creates one skill from its JSON payload.
pub async fn create_skill(
    State(app_state): State<AppState>,
    Json(request): Json<CreateSkillRequest>,
) -> Result<Json<CreateSkillResponse>, WebApiError> {
    app_state
        .backend()
        .create_skill(request)
        .map(Json)
        .map_err(Into::into)
}

/// Gets one skill identified by its path identifier.
pub async fn get_skill(
    State(app_state): State<AppState>,
    Path(path): Path<SkillPath>,
) -> Result<Json<GetSkillResponse>, WebApiError> {
    app_state
        .backend()
        .get_skill(GetSkillRequest {
            skill_id: path.skill_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Lists every visible skill.
pub async fn list_skills(
    State(app_state): State<AppState>,
) -> Result<Json<ListSkillsResponse>, WebApiError> {
    app_state
        .backend()
        .list_skills(ListSkillsRequest {})
        .map(Json)
        .map_err(Into::into)
}

/// Replaces one skill while using the URL identifier as its stable identity.
pub async fn update_skill(
    State(app_state): State<AppState>,
    Path(path): Path<SkillPath>,
    Json(body): Json<UpdateSkillBody>,
) -> Result<Json<UpdateSkillResponse>, WebApiError> {
    app_state
        .backend()
        .update_skill(UpdateSkillRequest {
            skill_id: path.skill_id,
            name: body.name,
            description: body.description,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Imports one uploaded skill folder from a multipart request, committing it atomically.
///
/// Each file part carries its skill-root-relative path as the part file name; non-file fields are
/// ignored so the application layer receives only the uploaded folder contents to stage.
pub async fn import_skill(
    State(app_state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<CreateSkillResponse>, WebApiError> {
    let mut files = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        let relative_path = match field.file_name() {
            Some(file_name) => file_name.to_string(),
            None => continue,
        };
        let bytes = field.bytes().await.map_err(multipart_error)?;
        files.push(UploadedSkillFile {
            relative_path,
            bytes: bytes.to_vec(),
        });
    }

    app_state
        .backend()
        .import_skill(files)
        .map(Json)
        .map_err(Into::into)
}

/// Converts multipart parsing and body-limit failures into the shared public error contract.
fn multipart_error(source: MultipartError) -> WebApiError {
    if source.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return WebApiError::with_source(
            ErrorClassification::PayloadTooLarge,
            PublicError::SkillUploadTooLarge(SkillUploadTooLargeParams {
                max_bytes: MAX_SKILL_UPLOAD_BYTES,
            }),
            "skill upload exceeds the request body limit",
            source,
        );
    }

    if source.status() == StatusCode::BAD_REQUEST {
        return WebApiError::with_source(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "failed to decode multipart skill upload",
            source,
        );
    }

    WebApiError::internal("failed to read multipart skill upload", source)
}

/// Soft-deletes one skill addressed by its URL identifier.
pub async fn delete_skill(
    State(app_state): State<AppState>,
    Path(path): Path<SkillPath>,
) -> Result<Json<DeleteSkillResponse>, WebApiError> {
    app_state
        .backend()
        .delete_skill(DeleteSkillRequest {
            skill_id: path.skill_id,
        })
        .map(Json)
        .map_err(Into::into)
}

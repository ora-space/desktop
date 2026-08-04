use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, UpdateSkillRequest,
    UpdateSkillResponse,
};
use serde::Deserialize;

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

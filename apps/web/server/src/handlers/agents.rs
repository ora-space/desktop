use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    CommitAgentImportRequest, CommitAgentImportResponse, CreateAgentRequest, CreateAgentResponse,
    DeleteAgentRequest, DeleteAgentResponse, GetAgentRequest, GetAgentResponse, ListAgentsRequest,
    ListAgentsResponse, PrepareAgentImportRequest, PrepareAgentImportResponse, UpdateAgentRequest,
    UpdateAgentResponse,
};
use serde::Deserialize;

/// Carries the path identifier used to address one configurable-agent resource.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPath {
    agent_id: String,
}

/// Carries an agent replacement payload before the path identifier is attached.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentBody {
    name: String,
    description: String,
    #[serde(default)]
    content: Option<String>,
}

/// Creates one configurable agent type from its JSON payload.
pub async fn create_agent(
    State(app_state): State<AppState>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<CreateAgentResponse>, WebApiError> {
    app_state
        .backend()
        .create_agent(request)
        .map(Json)
        .map_err(Into::into)
}

/// Gets one configurable agent type by its path identifier.
pub async fn get_agent(
    State(app_state): State<AppState>,
    Path(path): Path<AgentPath>,
) -> Result<Json<GetAgentResponse>, WebApiError> {
    app_state
        .backend()
        .get_agent(GetAgentRequest {
            agent_id: path.agent_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Lists every visible configurable agent type.
pub async fn list_agents(
    State(app_state): State<AppState>,
) -> Result<Json<ListAgentsResponse>, WebApiError> {
    app_state
        .backend()
        .list_agents(ListAgentsRequest {})
        .map(Json)
        .map_err(Into::into)
}

/// Replaces one configurable agent type while using the URL identifier as its stable identity.
pub async fn update_agent(
    State(app_state): State<AppState>,
    Path(path): Path<AgentPath>,
    Json(body): Json<UpdateAgentBody>,
) -> Result<Json<UpdateAgentResponse>, WebApiError> {
    app_state
        .backend()
        .update_agent(UpdateAgentRequest {
            agent_id: path.agent_id,
            name: body.name,
            description: body.description,
            content: body.content,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Soft-deletes one configurable agent type addressed by its URL identifier.
pub async fn delete_agent(
    State(app_state): State<AppState>,
    Path(path): Path<AgentPath>,
) -> Result<Json<DeleteAgentResponse>, WebApiError> {
    app_state
        .backend()
        .delete_agent(DeleteAgentRequest {
            agent_id: path.agent_id,
        })
        .map(Json)
        .map_err(Into::into)
}

/// Parses and previews one Agent Markdown document before import.
pub async fn prepare_agent_import(
    State(app_state): State<AppState>,
    Json(request): Json<PrepareAgentImportRequest>,
) -> Result<Json<PrepareAgentImportResponse>, WebApiError> {
    app_state
        .backend()
        .prepare_agent_import(request)
        .map(Json)
        .map_err(Into::into)
}

/// Commits one previously previewed Agent Markdown document with a frozen conflict decision.
pub async fn commit_agent_import(
    State(app_state): State<AppState>,
    Json(request): Json<CommitAgentImportRequest>,
) -> Result<Json<CommitAgentImportResponse>, WebApiError> {
    app_state
        .backend()
        .commit_agent_import(request)
        .map(Json)
        .map_err(Into::into)
}

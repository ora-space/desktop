use crate::app_state::AppState;
use crate::error::{DeferredCompletion, WebApiError, current_lifecycle};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Response, header};
use futures_util::stream;
use ora_backend::{BackendError, SessionEventStream};
use ora_contracts::{
    AgentCli, ContractError, CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest,
    DeleteSessionResponse, EmptyErrorParams, GetSessionRequest, GetSessionResponse,
    ListAgentModelsRequest, ListAgentModelsResponse, ListSessionsRequest, ListSessionsResponse,
    LoadSessionRequest, PromptSessionRequest, PublicError, RespondToPermissionRequest,
    RespondToPermissionResponse, ResumeSessionHistoryRequest, ResumeSessionHistoryResponse,
    StopSessionRequest, StopSessionResponse, SwitchSessionAgentRequest, SwitchSessionAgentResponse,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

/// Carries the request path segment used by session identifier routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPath {
    session_id: String,
}

/// Carries the structured prompt body after the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSessionBody {
    prompt: Vec<ora_contracts::acp::content::ContentBlock>,
}

/// Carries a permission selection while the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondToPermissionBody {
    permission_request_id: String,
    option_id: String,
}

/// Carries the target CLI while the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchSessionAgentBody {
    agent_cli: AgentCli,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamFrame<Event> {
    Data { data: Event },
    Error { error: ContractError },
    End,
}

/// Creates one provider-backed session after the ACP setup handshake succeeds.
pub async fn create_session(
    State(app_state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, WebApiError> {
    app_state
        .backend()
        .create_session(request)
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists models grouped by every CLI whose discovery command succeeds.
pub async fn list_agent_models(
    State(app_state): State<AppState>,
) -> Result<Json<ListAgentModelsResponse>, WebApiError> {
    app_state
        .backend()
        .list_agent_models(ListAgentModelsRequest {})
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Loads one persisted Ora session view.
pub async fn get_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<GetSessionResponse>, WebApiError> {
    app_state
        .backend()
        .get_session(GetSessionRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists every visible session by delegating to the persisted query API.
pub async fn list_sessions(
    State(app_state): State<AppState>,
) -> Result<Json<ListSessionsResponse>, WebApiError> {
    app_state
        .backend()
        .list_sessions(ListSessionsRequest {})
        .map(Json)
        .map_err(WebApiError::from)
}

/// Streams ACP history replay as private NDJSON transport frames.
pub async fn load_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Response<Body>, WebApiError> {
    let events = app_state
        .backend()
        .load_session(LoadSessionRequest {
            session_id: path.session_id,
        })
        .await
        .map_err(WebApiError::from)?;
    Ok(stream_response(events))
}

/// Streams one structured ACP prompt turn as private NDJSON transport frames.
pub async fn prompt_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<PromptSessionBody>,
) -> Result<Response<Body>, WebApiError> {
    let events = app_state
        .backend()
        .prompt_session(PromptSessionRequest {
            session_id: path.session_id,
            prompt: body.prompt,
        })
        .await
        .map_err(WebApiError::from)?;
    Ok(stream_response(events))
}

/// Routes one permission selection to the actor that owns the pending request.
pub async fn respond_to_permission(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<RespondToPermissionBody>,
) -> Result<Json<RespondToPermissionResponse>, WebApiError> {
    app_state
        .backend()
        .respond_to_session_permission(RespondToPermissionRequest {
            session_id: path.session_id,
            permission_request_id: body.permission_request_id,
            option_id: body.option_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Stops one provider process while preserving the session for a later load.
pub async fn stop_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<StopSessionResponse>, WebApiError> {
    app_state
        .backend()
        .stop_session(StopSessionRequest {
            session_id: path.session_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Moves one conversation onto a different agent CLI without changing its identity.
pub async fn switch_session_agent(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<SwitchSessionAgentBody>,
) -> Result<Json<SwitchSessionAgentResponse>, WebApiError> {
    app_state
        .backend()
        .switch_session_agent(SwitchSessionAgentRequest {
            session_id: path.session_id,
            agent_cli: body.agent_cli,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Returns a session whose history writes failed to a writable state.
pub async fn resume_session_history(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<ResumeSessionHistoryResponse>, WebApiError> {
    app_state
        .backend()
        .resume_session_history(ResumeSessionHistoryRequest {
            session_id: path.session_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Stops the runtime and removes the Ora-owned session record and its history.
pub async fn delete_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<DeleteSessionResponse>, WebApiError> {
    app_state
        .backend()
        .delete_session(DeleteSessionRequest {
            session_id: path.session_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Converts one backend event receiver into ordered, atomic NDJSON transport frames.
fn stream_response<Event>(events: SessionEventStream<Event>) -> Response<Body>
where
    Event: Serialize + Send + 'static,
{
    let lifecycle = current_lifecycle();
    let body_stream = stream::unfold(
        (events, false, lifecycle),
        |(mut events, ended, lifecycle)| async move {
            if ended {
                return None;
            }
            let (frame, next_ended) = match events.recv().await {
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
            };
            let mut bytes = serde_json::to_vec(&frame).unwrap_or_else(|source| {
                let error = BackendError::internal("failed to encode stream frame", source);
                lifecycle.complete_failure(&error);
                serde_json::to_vec(&StreamFrame::<Event>::Error {
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
                (events, next_ended, lifecycle),
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

use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Response, header};
use futures_util::stream;
use ora_backend::{BackendError, SessionEventStream};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListAgentModelsRequest, ListAgentModelsResponse,
    ListSessionsRequest, ListSessionsResponse, LoadSessionRequest, PromptSessionRequest,
    RespondToPermissionRequest, RespondToPermissionResponse, SetSessionModeRequest,
    SetSessionModeResponse, StopSessionRequest, StopSessionResponse,
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

/// Carries the selected provider mode after the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionModeBody {
    mode_id: ora_contracts::acp::session_mode::SessionModeId,
}

/// Carries a permission selection while the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondToPermissionBody {
    permission_request_id: String,
    option_id: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamFrame<Event> {
    Data { data: Event },
    Error { error: BackendError },
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

/// Changes the provider mode for one running session.
pub async fn set_session_mode(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<SetSessionModeBody>,
) -> Result<Json<SetSessionModeResponse>, WebApiError> {
    app_state
        .backend()
        .set_session_mode(SetSessionModeRequest {
            session_id: path.session_id,
            mode_id: body.mode_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
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

/// Stops the runtime and removes only the Ora-owned session record.
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
    let body_stream = stream::unfold((events, false), |(mut events, ended)| async move {
        if ended {
            return None;
        }
        let (frame, next_ended) = match events.recv().await {
            Some(Ok(event)) => (StreamFrame::Data { data: event }, false),
            Some(Err(error)) => (StreamFrame::Error { error }, true),
            None => (StreamFrame::End, true),
        };
        let mut bytes = serde_json::to_vec(&frame).unwrap_or_else(|_| {
            b"{\"type\":\"error\",\"error\":{\"code\":\"stream_encoding_failed\",\"message\":\"failed to encode stream frame\"}}".to_vec()
        });
        bytes.push(b'\n');
        Some((
            Ok::<Bytes, Infallible>(Bytes::from(bytes)),
            (events, next_ended),
        ))
    });
    let mut response = Response::new(Body::from_stream(body_stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    response
}

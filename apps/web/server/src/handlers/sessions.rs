use crate::app_state::AppState;
use crate::error::{DeferredCompletion, WebApiError, current_lifecycle};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, Response, header};
use futures_util::stream;
use ora_backend::{BackendError, SessionEventStream};
use ora_contracts::{
    AgentCli, AttachSessionRequest, AttachSessionResponse, ContractError, DeleteSessionRequest,
    DeleteSessionResponse, EmptyErrorParams, GetAgentRuntimeStatusRequest,
    GetAgentRuntimeStatusResponse, GetSessionRequest, GetSessionResponse, ListSessionsRequest,
    ListSessionsResponse, LoadSessionRequest, PromptSessionRequest, PublicError,
    RespondToPermissionRequest, RespondToPermissionResponse, ResumeSessionHistoryRequest,
    ResumeSessionHistoryResponse, SetSessionConfigRequest, SetSessionConfigResponse,
    StopSessionRequest, StopSessionResponse, SwitchSessionAgentRequest, SwitchSessionAgentResponse,
    WarmSessionRequest, WarmSessionResponse, WatchAppEventsRequest,
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
    prompt: Vec<agent_client_protocol_schema::v1::ContentBlock>,
}

/// Carries a permission selection while the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondToPermissionBody {
    permission_request_id: String,
    option_id: String,
}

/// Carries the target CLI and claiming client while the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchSessionAgentBody {
    agent_cli: AgentCli,
    client_id: String,
}

/// Carries one configuration change while the path owns the Ora session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionConfigBody {
    config_id: String,
    value: String,
}

/// Carries the owning Task while the path owns the warm session identifier.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachSessionBody {
    task_id: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamFrame<Event> {
    Data { data: Event },
    Error { error: ContractError },
    End,
}

/// Returns the warm provider session backing one chat surface.
pub async fn warm_session(
    State(app_state): State<AppState>,
    Json(request): Json<WarmSessionRequest>,
) -> Result<Json<WarmSessionResponse>, WebApiError> {
    app_state
        .backend()
        .warm_session(request)
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Applies one configuration option to a warm or persisted session.
pub async fn set_session_config(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<SetSessionConfigBody>,
) -> Result<Json<SetSessionConfigResponse>, WebApiError> {
    app_state
        .backend()
        .set_session_config(SetSessionConfigRequest {
            session_id: path.session_id,
            config_id: body.config_id,
            value: body.value,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Persists one warm session against the Task that now owns it.
pub async fn attach_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<AttachSessionBody>,
) -> Result<Json<AttachSessionResponse>, WebApiError> {
    app_state
        .backend()
        .attach_session(AttachSessionRequest {
            session_id: path.session_id,
            task_id: body.task_id,
        })
        .await
        .map(Json)
        .map_err(WebApiError::from)
}

/// Reports the live detection status of every application-scoped CLI runtime.
pub async fn get_agent_runtime_status(
    State(app_state): State<AppState>,
) -> Result<Json<GetAgentRuntimeStatusResponse>, WebApiError> {
    app_state
        .backend()
        .get_agent_runtime_status(GetAgentRuntimeStatusRequest {})
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

/// Watches application invalidations using the shared NDJSON contract stream.
pub async fn watch_app_events(
    State(app_state): State<AppState>,
    Json(request): Json<WatchAppEventsRequest>,
) -> Result<Response<Body>, WebApiError> {
    let events = app_state
        .backend()
        .watch_app_events(request)
        .await
        .map_err(WebApiError::from)?;
    Ok(stream_response_with_heartbeat(events))
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
            client_id: body.client_id,
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
    stream_response_inner(events, None)
}

/// Converts an application event receiver into NDJSON frames with transport-only heartbeats.
fn stream_response_with_heartbeat<Event>(events: SessionEventStream<Event>) -> Response<Body>
where
    Event: Serialize + Send + 'static,
{
    stream_response_inner(events, Some(std::time::Duration::from_secs(15)))
}

/// Builds one NDJSON response and optionally emits empty-line heartbeats while it is idle.
fn stream_response_inner<Event>(
    events: SessionEventStream<Event>,
    heartbeat_period: Option<std::time::Duration>,
) -> Response<Body>
where
    Event: Serialize + Send + 'static,
{
    let lifecycle = current_lifecycle();
    let heartbeat = heartbeat_period
        .map(|period| tokio::time::interval_at(tokio::time::Instant::now() + period, period));
    let body_stream = stream::unfold(
        (events, false, lifecycle, heartbeat),
        |(mut events, ended, lifecycle, mut heartbeat)| async move {
            if ended {
                return None;
            }
            let next_event = match heartbeat.as_mut() {
                Some(interval) => tokio::select! {
                    biased;
                    event = events.recv() => Some(event),
                    _ = interval.tick() => None,
                },
                None => Some(events.recv().await),
            };
            let Some(event) = next_event else {
                return Some((
                    Ok::<Bytes, Infallible>(Bytes::from_static(b"\n")),
                    (events, false, lifecycle, heartbeat),
                ));
            };
            let (frame, next_ended) = match event {
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
                (events, next_ended, lifecycle, heartbeat),
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

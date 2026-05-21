use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    SessionStatus, TerminalClientMessage, TerminalServerMessage, UpdateSessionRequest,
    UpdateSessionResponse,
};
use ora_pty::PtyOutputChunkEvent;
use serde::Deserialize;

/// Carries the request path segment used by session identifier routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPath {
    session_id: String,
}

/// Carries the HTTP body used for session update routes before the path identifier is applied.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionBody {
    task_id: String,
    agent_id: String,
    agent_session_id: Option<String>,
    status: SessionStatus,
}

/// Creates one session by forwarding the request body into the application layer.
pub async fn create_session(
    State(app_state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .create_session(request)
        .map(Json)
        .map_err(WebApiError::from)
}

/// Loads one session by combining the path identifier into the contract request.
pub async fn get_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<GetSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .get_session(GetSessionRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Lists every visible session by delegating to the application handler.
pub async fn list_sessions(
    State(app_state): State<AppState>,
) -> Result<Json<ListSessionsResponse>, WebApiError> {
    app_state
        .session_api()
        .list_sessions(ListSessionsRequest {})
        .map(Json)
        .map_err(WebApiError::from)
}

/// Replaces one session by combining the route identifier with the JSON body payload.
pub async fn update_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
    Json(body): Json<UpdateSessionBody>,
) -> Result<Json<UpdateSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .update_session(UpdateSessionRequest {
            session_id: path.session_id,
            task_id: body.task_id,
            agent_id: body.agent_id,
            agent_session_id: body.agent_session_id,
            status: body.status,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Deletes one session by combining the path identifier into the contract request.
pub async fn delete_session(
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Json<DeleteSessionResponse>, WebApiError> {
    app_state
        .session_api()
        .delete_session(DeleteSessionRequest {
            session_id: path.session_id,
        })
        .map(Json)
        .map_err(WebApiError::from)
}

/// Attaches one WebSocket client to a running terminal session after validating the runtime state.
pub async fn attach_terminal_session(
    ws: WebSocketUpgrade,
    State(app_state): State<AppState>,
    Path(path): Path<SessionPath>,
) -> Result<Response, WebApiError> {
    let session_id = path.session_id;
    let attachment = app_state
        .session_api()
        .attach_terminal_session(session_id.clone())
        .map_err(WebApiError::from)?;
    let upgraded_app_state = app_state.clone();

    Ok(ws.on_upgrade(move |socket| {
        serve_terminal_socket(upgraded_app_state, session_id, attachment, socket)
    }))
}

/// Drives the first-slice terminal protocol over one attached WebSocket connection.
async fn serve_terminal_socket(
    app_state: AppState,
    session_id: String,
    mut attachment: ora_application::TerminalAttachment,
    mut socket: WebSocket,
) {
    if !send_terminal_message(
        &mut socket,
        &TerminalServerMessage::Ready {
            session_id: session_id.clone(),
        },
    )
    .await
    {
        let _ = app_state.session_api().detach_terminal_session(&session_id);
        return;
    }

    for data in &attachment.replay {
        if !send_terminal_message(
            &mut socket,
            &TerminalServerMessage::History { data: data.clone() },
        )
        .await
        {
            let _ = app_state.session_api().detach_terminal_session(&session_id);
            return;
        }
    }

    loop {
        tokio::select! {
            _ = attachment.session_token.cancelled() => {
                break;
            }
            maybe_event = attachment.output_receiver.recv() => {
                match maybe_event {
                    Ok(PtyOutputChunkEvent::Output { data }) => {
                        if !send_terminal_message(&mut socket, &TerminalServerMessage::Output { data }).await {
                            break;
                        }
                    }
                    Ok(PtyOutputChunkEvent::Exit { exit_code }) => {
                        let _ = send_terminal_message(&mut socket, &TerminalServerMessage::Exit { exit_code }).await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            maybe_message = socket.recv() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        if !handle_terminal_client_message(&app_state, &session_id, &mut socket, &text).await {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => break,
                }
            }
        }
    }

    let _ = app_state.session_api().detach_terminal_session(&session_id);
}

/// Handles one terminal client message and reports whether the socket loop should continue.
async fn handle_terminal_client_message(
    app_state: &AppState,
    session_id: &str,
    socket: &mut WebSocket,
    text: &str,
) -> bool {
    let message = match serde_json::from_str::<TerminalClientMessage>(text) {
        Ok(message) => message,
        Err(error) => {
            return send_terminal_message(
                socket,
                &TerminalServerMessage::Error {
                    code: "bad_terminal_message".to_string(),
                    message: error.to_string(),
                },
            )
            .await;
        }
    };
    let result = match message {
        TerminalClientMessage::Input { data } => app_state
            .session_api()
            .send_terminal_input(session_id.to_string(), data),
        TerminalClientMessage::Resize { cols, rows } => app_state
            .session_api()
            .resize_terminal_session(session_id.to_string(), cols, rows),
        TerminalClientMessage::Kill {} => app_state
            .session_api()
            .kill_terminal_session(session_id.to_string()),
    };

    match result {
        Ok(()) => true,
        Err(error) => {
            let should_continue = !matches!(
                error,
                ora_application::ApplicationError::TerminalRuntimeMissing { .. }
                    | ora_application::ApplicationError::TerminalSessionStopped { .. }
            );

            send_terminal_message(
                socket,
                &TerminalServerMessage::Error {
                    code: terminal_error_code(&error).to_string(),
                    message: error.to_string(),
                },
            )
            .await
                && should_continue
        }
    }
}

/// Sends one terminal protocol message as JSON text and reports whether the send succeeded.
async fn send_terminal_message(socket: &mut WebSocket, message: &TerminalServerMessage) -> bool {
    let payload = match serde_json::to_string(message) {
        Ok(payload) => payload,
        Err(_) => return false,
    };

    socket.send(Message::Text(payload.into())).await.is_ok()
}

/// Converts a stable application error into the terminal protocol error code field.
fn terminal_error_code(error: &ora_application::ApplicationError) -> &'static str {
    match error {
        ora_application::ApplicationError::TerminalStartup { .. } => "terminal_startup_error",
        ora_application::ApplicationError::TerminalRuntimeMissing { .. } => {
            "terminal_runtime_missing"
        }
        ora_application::ApplicationError::TerminalAlreadyAttached { .. } => {
            "terminal_already_attached"
        }
        ora_application::ApplicationError::TerminalSessionNotTerminal { .. } => {
            "terminal_session_not_terminal"
        }
        ora_application::ApplicationError::TerminalSessionStopped { .. } => {
            "terminal_session_stopped"
        }
        ora_application::ApplicationError::InvalidTerminalRequest { .. } => {
            "invalid_terminal_request"
        }
        ora_application::ApplicationError::SessionNotFound { .. } => "session_not_found",
        ora_application::ApplicationError::SessionRepository { .. } => "session_repository_error",
        ora_application::ApplicationError::TaskNotFound { .. } => "task_not_found",
        ora_application::ApplicationError::TaskRepository { .. } => "task_repository_error",
        ora_application::ApplicationError::TaskWorktree { .. } => "task_worktree_error",
        ora_application::ApplicationError::WorktreeNotFound { .. } => "worktree_not_found",
        ora_application::ApplicationError::WorktreeRepository { .. } => "worktree_repository_error",
        ora_application::ApplicationError::ProjectNotFound { .. } => "project_not_found",
        ora_application::ApplicationError::ProjectRepository { .. } => "project_repository_error",
        ora_application::ApplicationError::ProjectOccupied { .. } => "project_occupied",
        ora_application::ApplicationError::ProjectWorkContextNotFound { .. } => {
            "project_work_context_not_found"
        }
        ora_application::ApplicationError::ProjectWorkContextRepository { .. } => {
            "project_work_context_repository_error"
        }
    }
}

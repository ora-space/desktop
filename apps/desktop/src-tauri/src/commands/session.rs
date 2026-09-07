//! Desktop session operations.

use crate::{error::CommandError, state::DesktopState};
use ora_contracts::*;
use tauri::State;

/// Starts history replay without embedding session creation in the shared transport dispatcher.
pub(super) async fn start_load(
    state: State<'_, DesktopState>,
    request: LoadSessionRequest,
    context: super::stream::StreamStart,
) -> Result<(), CommandError> {
    context.events(state.backend.sessions().load(request)).await
}

/// Starts a prompt using the session-owned backend while the context owns stream cancellation.
pub(super) async fn start_prompt(
    state: State<'_, DesktopState>,
    request: PromptSessionRequest,
    context: super::stream::StreamStart,
) -> Result<(), CommandError> {
    context
        .events(state.backend.sessions().prompt(request))
        .await
}

async_backend_command!(
    start_session,
    StartSessionRequest,
    StartSessionResponse,
    sessions.start,
    "Executes start_session through the session-owned interface."
);
async_backend_command!(
    set_session_config,
    SetSessionConfigRequest,
    SetSessionConfigResponse,
    sessions.set_config,
    "Executes set_session_config through the session-owned interface."
);
backend_command!(
    get_session,
    GetSessionRequest,
    GetSessionResponse,
    sessions.get,
    "Executes get_session through the session-owned interface."
);
backend_command!(
    list_sessions,
    ListSessionsRequest,
    ListSessionsResponse,
    sessions.list,
    "Executes list_sessions through the session-owned interface."
);
async_backend_command!(
    respond_to_session_permission,
    RespondToPermissionRequest,
    RespondToPermissionResponse,
    sessions.respond_to_permission,
    "Executes respond_to_session_permission through the session-owned interface."
);
async_backend_command!(
    stop_session,
    StopSessionRequest,
    StopSessionResponse,
    sessions.stop,
    "Executes stop_session through the session-owned interface."
);
backend_command!(
    cancel_session_prompt,
    CancelSessionPromptRequest,
    CancelSessionPromptResponse,
    sessions.cancel_prompt,
    "Executes cancel_session_prompt through the session-owned interface."
);
async_backend_command!(
    switch_session_agent,
    SwitchSessionAgentRequest,
    SwitchSessionAgentResponse,
    sessions.switch_agent,
    "Executes switch_session_agent through the session-owned interface."
);
async_backend_command!(
    resume_session_history,
    ResumeSessionHistoryRequest,
    ResumeSessionHistoryResponse,
    sessions.resume_history,
    "Executes resume_session_history through the session-owned interface."
);
async_backend_command!(
    delete_session,
    DeleteSessionRequest,
    DeleteSessionResponse,
    sessions.delete,
    "Executes delete_session through the session-owned interface."
);
async_backend_command!(
    rename_session,
    RenameSessionRequest,
    RenameSessionResponse,
    sessions.rename,
    "Executes rename_session through the session-owned interface."
);

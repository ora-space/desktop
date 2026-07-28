use crate::acp::content::ContentBlock;
use crate::acp::permission::PermissionOption;
use crate::acp::prompt::StopReason;
use crate::acp::session::SessionUpdate;
use crate::acp::session_config_options::SessionConfigOption;
use crate::acp::slash_command::AvailableCommand;
use crate::acp::tool_call::ToolCallUpdate;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies the shared CLI runtime selected for a provider-backed session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "session.ts")]
pub enum AgentCli {
    OpenCode,
    Nga,
    CodeAgentCli,
}

/// Describes whether a persisted session is registered on its shared CLI connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub enum SessionStatus {
    Running,
    Stopped,
}

/// Reports whether Ora can still extend this session's recorded history.
///
/// Separate from [`SessionStatus`] on purpose: that says whether the conversation
/// is registered on a CLI connection, this says whether the record of it can
/// still grow. A running session whose disk filled is both at once, and the user
/// has to be told which one broke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub enum SessionHistoryState {
    Writable,
    /// A write failed; the session refuses prompts until its history is resumed.
    Degraded {
        reason: String,
    },
}

/// Describes the public session payload without exposing the provider session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct Session {
    pub id: String,
    pub task_id: String,
    /// The CLI this conversation currently runs on, which switching replaces.
    pub agent_cli: AgentCli,
    pub status: SessionStatus,
    pub history_state: SessionHistoryState,
}

/// Selects the working directory one warm session is created against.
///
/// The two variants mirror how Ora resolves a cwd: an existing Task owns either
/// a linked worktree or the project root, while a chat whose Task does not exist
/// yet can only target the project root. Modelling this as an enum keeps callers
/// from having to pass two optional identifiers and guess which one wins.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub enum WarmSessionTarget {
    Task {
        #[serde(rename = "taskId")]
        task_id: String,
    },
    ProjectRoot {
        #[serde(rename = "projectId")]
        project_id: String,
    },
}

/// Requests the reusable warm provider session backing one chat surface.
///
/// The request carries no cwd: the backend derives it from `target` on every
/// call, so a worktree that moved or was recreated invalidates the warm entry
/// instead of silently addressing a stale directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct WarmSessionRequest {
    pub target: WarmSessionTarget,
    pub agent_cli: AgentCli,
    /// Identifies the client surface that will own the returned session.
    ///
    /// Warm entries are keyed by this value because one backend can serve
    /// several clients (browser tabs against the Web server). Without it two
    /// tabs showing the same selection would share one provider session, and
    /// whichever attached first would take the other tab's conversation.
    pub client_id: String,
}

/// Returns the warm session identifier together with the agent's current configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct WarmSessionResponse {
    /// The final Ora session id. It is not persisted until `attachSession`
    /// succeeds, so `getSession` and `listSessions` do not report it yet.
    pub session_id: String,
    pub config_options: Vec<SessionConfigOption>,
}

/// Sets one selectable configuration option on a warm or persisted session.
///
/// `value` is the chosen option's value id. Only id-valued options are
/// expressible because Ora does not advertise the boolean config-option client
/// capability, so an agent never offers a value this request cannot carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct SetSessionConfigRequest {
    pub session_id: String,
    pub config_id: String,
    pub value: String,
}

/// Returns the full option set after the agent applies the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct SetSessionConfigResponse {
    pub config_options: Vec<SessionConfigOption>,
}

/// Binds one warm session to its owning Task and persists the Ora record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct AttachSessionRequest {
    pub session_id: String,
    pub task_id: String,
}

/// Returns the newly persisted session payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct AttachSessionResponse {
    pub session: Session,
    pub available_commands: Vec<AvailableCommand>,
}

/// Identifies which session to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct GetSessionRequest {
    pub session_id: String,
}

/// Returns one session payload after a successful fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct GetSessionResponse {
    pub session: Session,
}

/// Requests the full visible session list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct ListSessionsRequest {}

/// Returns the visible session list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct ListSessionsResponse {
    pub sessions: Vec<Session>,
}

/// Identifies a stopped session whose provider history should be replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct LoadSessionRequest {
    pub session_id: String,
}

/// Carries one or more ACP content blocks to the provider session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct PromptSessionRequest {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

/// Exposes an opaque permission request while preserving the agent's typed option payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct SessionPermissionRequest {
    pub permission_request_id: String,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
}

/// Replays Ora's recorded history while keeping JSON-RPC framing private to the backend.
///
/// The stream carries assembled updates read back from Ora's own record, not the
/// provider's replay. `TurnEnded` has no ACP equivalent and exists because a
/// cancelled turn would otherwise be indistinguishable from a completed one —
/// information provider replay never carried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "session.ts")]
pub enum LoadSessionEvent {
    SessionUpdate {
        update: SessionUpdate,
    },
    PermissionRequest(SessionPermissionRequest),
    TurnEnded {
        #[serde(rename = "stopReason")]
        stop_reason: StopReason,
    },
    Completed,
}

/// Streams one prompt turn and ends with the provider's typed stop reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "session.ts")]
pub enum PromptSessionEvent {
    SessionUpdate {
        update: SessionUpdate,
    },
    PermissionRequest(SessionPermissionRequest),
    Completed {
        #[serde(rename = "stopReason")]
        stop_reason: StopReason,
    },
}

/// Selects one option for a still-pending permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct RespondToPermissionRequest {
    pub session_id: String,
    pub permission_request_id: String,
    pub option_id: String,
}

/// Confirms that a permission response was delivered to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct RespondToPermissionResponse {}

/// Identifies a running session whose child process should be stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct StopSessionRequest {
    pub session_id: String,
}

/// Returns the stopped public session snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct StopSessionResponse {
    pub session: Session,
}

/// Moves one existing conversation onto a different agent CLI.
///
/// Only the binding changes: the session keeps its identifier, its task, and the
/// history it has accumulated. The new CLI starts with no context, so Ora's
/// recorded transcript is prepended to the next prompt sent into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct SwitchSessionAgentRequest {
    pub session_id: String,
    pub agent_cli: AgentCli,
}

/// Returns the session rebound to its new CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct SwitchSessionAgentResponse {
    pub session: Session,
    pub available_commands: Vec<AvailableCommand>,
}

/// Returns a session whose history writes failed to a writable state.
///
/// Resuming appends a record of what went missing before accepting new content,
/// so the conversation never contains a gap that cannot be seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct ResumeSessionHistoryRequest {
    pub session_id: String,
}

/// Returns the session after its history became writable again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct ResumeSessionHistoryResponse {
    pub session: Session,
}

/// Identifies which Ora session record to remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct DeleteSessionRequest {
    pub session_id: String,
}

/// Returns the removed Ora session identifier without deleting provider history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "session.ts")]
pub struct DeleteSessionResponse {
    pub session_id: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    AgentCli::export(config)?;
    SessionStatus::export(config)?;
    SessionHistoryState::export(config)?;
    Session::export(config)?;
    SwitchSessionAgentRequest::export(config)?;
    SwitchSessionAgentResponse::export(config)?;
    ResumeSessionHistoryRequest::export(config)?;
    ResumeSessionHistoryResponse::export(config)?;
    WarmSessionTarget::export(config)?;
    WarmSessionRequest::export(config)?;
    WarmSessionResponse::export(config)?;
    SetSessionConfigRequest::export(config)?;
    SetSessionConfigResponse::export(config)?;
    AttachSessionRequest::export(config)?;
    AttachSessionResponse::export(config)?;
    GetSessionRequest::export(config)?;
    GetSessionResponse::export(config)?;
    ListSessionsRequest::export(config)?;
    ListSessionsResponse::export(config)?;
    LoadSessionRequest::export(config)?;
    PromptSessionRequest::export(config)?;
    SessionPermissionRequest::export(config)?;
    LoadSessionEvent::export(config)?;
    PromptSessionEvent::export(config)?;
    RespondToPermissionRequest::export(config)?;
    RespondToPermissionResponse::export(config)?;
    StopSessionRequest::export(config)?;
    StopSessionResponse::export(config)?;
    DeleteSessionRequest::export(config)?;
    DeleteSessionResponse::export(config)?;
    Ok(())
}

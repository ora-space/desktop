use std::{path::PathBuf, sync::Arc};

use super::{
    common::SessionId,
    content::ContentBlock,
    mcp::McpServer,
    plan::Plan,
    serde_util::{IntoMaybeUndefined, IntoOption, MaybeUndefined},
    session_config_options::SessionConfigOption,
    session_mode::{SessionModeId, SessionModeState},
    slash_command::AvailableCommand,
    tool_call::{ToolCall, ToolCallUpdate},
};
use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

/// Unique identifier for a message within a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Display, From, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(export_to = "acp/session.ts")]
pub struct MessageId(pub Arc<str>);

impl MessageId {
    /// Wraps a protocol string as a typed [`MessageId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

impl IntoOption<MessageId> for &str {
    fn into_option(self) -> Option<MessageId> {
        Some(MessageId::new(self))
    }
}

/// Request parameters for creating a new session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct NewSessionRequest {
    /// The working directory for this session. Must be an absolute path.
    pub cwd: PathBuf,
    /// Additional workspace roots for this session. Each path must be absolute.
    ///
    /// These expand the session's filesystem scope without changing `cwd`, which
    /// remains the base for relative paths. When omitted or empty, no
    /// additional roots are activated for the new session.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<PathBuf>,
    /// List of MCP (Model Context Protocol) servers the agent should connect to.
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub mcp_servers: Vec<McpServer>,
}

impl NewSessionRequest {
    /// Builds [`NewSessionRequest`] with the required request fields set
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            additional_directories: vec![],
            mcp_servers: vec![],
        }
    }

    /// Additional workspace roots for this session. Each path must be absolute.
    #[must_use]
    pub fn additional_directories(mut self, additional_directories: Vec<PathBuf>) -> Self {
        self.additional_directories = additional_directories;
        self
    }

    /// List of MCP (Model Context Protocol) servers the agent should connect to.
    #[must_use]
    pub fn mcp_servers(mut self, mcp_servers: Vec<McpServer>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }
}

/// Response from creating a new session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    /// Unique identifier for the created session.
    ///
    /// Used in all subsequent requests for this conversation.
    pub session_id: SessionId,
    /// Initial mode state if supported by the Agent
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub modes: Option<SessionModeState>,
    /// Initial session configuration options if supported by the Agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

impl NewSessionResponse {
    /// Builds [`NewSessionResponse`] with the required response fields set
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>) -> Self {
        Self {
            session_id: session_id.into(),
            modes: None,
            config_options: None,
        }
    }

    /// Initial mode state if supported by the Agent
    #[must_use]
    pub fn modes(mut self, modes: impl IntoOption<SessionModeState>) -> Self {
        self.modes = modes.into_option();
        self
    }

    /// Initial session configuration options if supported by the Agent.
    #[must_use]
    pub fn config_options(
        mut self,
        config_options: impl IntoOption<Vec<SessionConfigOption>>,
    ) -> Self {
        self.config_options = config_options.into_option();
        self
    }
}

/// Request parameters for loading an existing session.
///
/// Only available if the Agent supports the `loadSession` capability.
///
/// See protocol docs: [Loading Sessions](https://agentclientprotocol.com/protocol/session-setup#loading-sessions)
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionRequest {
    /// List of MCP servers to connect to for this session.
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub mcp_servers: Vec<McpServer>,
    /// The working directory for this session. Must be an absolute path.
    pub cwd: PathBuf,
    /// Additional workspace roots to activate for this session. Each path must be absolute.
    ///
    /// When omitted or empty, no additional roots are activated. When non-empty,
    /// this is the complete resulting additional-root list for the loaded
    /// session. It may differ from any previously used or reported list as long as
    /// the request `cwd` matches the session's `cwd`.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<PathBuf>,
    /// The ID of the session to load.
    pub session_id: SessionId,
}

impl LoadSessionRequest {
    /// Builds [`LoadSessionRequest`] with the required request fields set
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            mcp_servers: vec![],
            cwd: cwd.into(),
            additional_directories: vec![],
            session_id: session_id.into(),
        }
    }

    /// Additional workspace roots to activate for this session. Each path must be absolute.
    #[must_use]
    pub fn additional_directories(mut self, additional_directories: Vec<PathBuf>) -> Self {
        self.additional_directories = additional_directories;
        self
    }

    /// List of MCP servers to connect to for this session.
    #[must_use]
    pub fn mcp_servers(mut self, mcp_servers: Vec<McpServer>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }
}

/// Response from loading an existing session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionResponse {
    /// Initial mode state if supported by the Agent
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub modes: Option<SessionModeState>,
    /// Initial session configuration options if supported by the Agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

impl LoadSessionResponse {
    /// Builds [`LoadSessionResponse`] with the required response fields set
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Initial mode state if supported by the Agent
    #[must_use]
    pub fn modes(mut self, modes: impl IntoOption<SessionModeState>) -> Self {
        self.modes = modes.into_option();
        self
    }

    /// Initial session configuration options if supported by the Agent.
    #[must_use]
    pub fn config_options(
        mut self,
        config_options: impl IntoOption<Vec<SessionConfigOption>>,
    ) -> Self {
        self.config_options = config_options.into_option();
        self
    }
}

/// Request parameters for resuming an existing session.
///
/// Resumes an existing session without returning previous messages (unlike `session/load`).
/// This is useful for agents that can resume sessions but don't implement full session loading.
///
/// Only available if the Agent supports the `sessionCapabilities.resume` capability.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct ResumeSessionRequest {
    /// The ID of the session to resume.
    pub session_id: SessionId,
    /// The working directory for this session. Must be an absolute path.
    pub cwd: PathBuf,
    /// Additional workspace roots to activate for this session. Each path must be absolute.
    ///
    /// When omitted or empty, no additional roots are activated. When non-empty,
    /// this is the complete resulting additional-root list for the resumed
    /// session. It may differ from any previously used or reported list as long as
    /// the request `cwd` matches the session's `cwd`.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<PathBuf>,
    /// List of MCP servers to connect to for this session.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServer>,
}

impl ResumeSessionRequest {
    /// Builds [`ResumeSessionRequest`] with the required request fields set
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            cwd: cwd.into(),
            additional_directories: vec![],
            mcp_servers: vec![],
        }
    }

    /// Additional workspace roots to activate for this session. Each path must be absolute.
    #[must_use]
    pub fn additional_directories(mut self, additional_directories: Vec<PathBuf>) -> Self {
        self.additional_directories = additional_directories;
        self
    }

    /// List of MCP servers to connect to for this session.
    #[must_use]
    pub fn mcp_servers(mut self, mcp_servers: Vec<McpServer>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }
}

/// Response from resuming an existing session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct ResumeSessionResponse {
    /// Initial mode state if supported by the Agent
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub modes: Option<SessionModeState>,
    /// Initial session configuration options if supported by the Agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

impl ResumeSessionResponse {
    /// Builds [`ResumeSessionResponse`] with the required response fields set
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Initial mode state if supported by the Agent
    #[must_use]
    pub fn modes(mut self, modes: impl IntoOption<SessionModeState>) -> Self {
        self.modes = modes.into_option();
        self
    }

    /// Initial session configuration options if supported by the Agent.
    #[must_use]
    pub fn config_options(
        mut self,
        config_options: impl IntoOption<Vec<SessionConfigOption>>,
    ) -> Self {
        self.config_options = config_options.into_option();
        self
    }
}

/// Request parameters for closing an active session.
///
/// If supported, the agent **must** cancel any ongoing work related to the session
/// (treat it as if `session/cancel` was called) and then free up any resources
/// associated with the session.
///
/// Only available if the Agent supports the `sessionCapabilities.close` capability.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionRequest {
    /// The ID of the session to close.
    pub session_id: SessionId,
}

impl CloseSessionRequest {
    /// Builds [`CloseSessionRequest`] with the required request fields set
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

/// Response from closing a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
pub struct CloseSessionResponse {}

impl CloseSessionResponse {
    /// Builds [`CloseSessionResponse`] with the required response fields set
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request parameters for listing existing sessions.
///
/// Only available if the Agent supports the `sessionCapabilities.list` capability.
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsRequest {
    /// Filter sessions by working directory. Must be an absolute path.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Opaque cursor token from a previous response's nextCursor field for cursor-based pagination
    #[serde(default)]
    pub cursor: Option<String>,
}

impl ListSessionsRequest {
    /// Builds [`ListSessionsRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter sessions by working directory. Must be an absolute path.
    #[must_use]
    pub fn cwd(mut self, cwd: impl IntoOption<PathBuf>) -> Self {
        self.cwd = cwd.into_option();
        self
    }

    /// Opaque cursor token from a previous response's nextCursor field for cursor-based pagination
    #[must_use]
    pub fn cursor(mut self, cursor: impl IntoOption<String>) -> Self {
        self.cursor = cursor.into_option();
        self
    }
}

/// Response from listing sessions.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsResponse {
    /// Array of session information objects
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub sessions: Vec<SessionInfo>,
    /// Opaque cursor token. If present, pass this in the next request's cursor parameter
    /// to fetch the next page. If absent, there are no more results.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub next_cursor: Option<String>,
}

impl ListSessionsResponse {
    /// Builds [`ListSessionsResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        Self {
            sessions,
            next_cursor: None,
        }
    }

    /// Sets or clears the optional `nextCursor` field.
    #[must_use]
    pub fn next_cursor(mut self, next_cursor: impl IntoOption<String>) -> Self {
        self.next_cursor = next_cursor.into_option();
        self
    }
}

/// Request parameters for deleting an existing session from `session/list`.
///
/// Only available if the Agent supports the `sessionCapabilities.delete` capability.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionRequest {
    /// The ID of the session to delete.
    pub session_id: SessionId,
}

impl DeleteSessionRequest {
    /// Builds [`DeleteSessionRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

/// Response from deleting a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/session.ts")]
pub struct DeleteSessionResponse {}

impl DeleteSessionResponse {
    /// Builds [`DeleteSessionResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Information about a session returned by session/list
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// Unique identifier for the session
    pub session_id: SessionId,
    /// The working directory for this session. Must be an absolute path.
    pub cwd: PathBuf,
    /// Additional workspace roots reported for this session. Each path must be absolute.
    ///
    /// When present, this is the complete ordered additional-root list reported
    /// by the Agent. Omitted and empty values are equivalent: the response
    /// reports no additional roots.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<PathBuf>,

    /// Human-readable title for the session
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub title: Option<String>,
    /// ISO 8601 timestamp of last activity
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl SessionInfo {
    /// Builds [`SessionInfo`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            cwd: cwd.into(),
            additional_directories: vec![],
            title: None,
            updated_at: None,
        }
    }

    /// Additional workspace roots reported for this session. Each path must be absolute.
    #[must_use]
    pub fn additional_directories(mut self, additional_directories: Vec<PathBuf>) -> Self {
        self.additional_directories = additional_directories;
        self
    }

    /// Human-readable title for the session
    #[must_use]
    pub fn title(mut self, title: impl IntoOption<String>) -> Self {
        self.title = title.into_option();
        self
    }

    /// ISO 8601 timestamp of last activity
    #[must_use]
    pub fn updated_at(mut self, updated_at: impl IntoOption<String>) -> Self {
        self.updated_at = updated_at.into_option();
        self
    }
}

/// Different types of updates that can be sent during session processing.
///
/// These updates provide real-time feedback about the agent's progress.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    /// A chunk of the user's message being streamed.
    UserMessageChunk(ContentChunk),
    /// A chunk of the agent's response being streamed.
    AgentMessageChunk(ContentChunk),
    /// A chunk of the agent's internal reasoning being streamed.
    AgentThoughtChunk(ContentChunk),
    /// Notification that a new tool call has been initiated.
    ToolCall(ToolCall),
    /// Update on the status or results of a tool call.
    ToolCallUpdate(ToolCallUpdate),
    /// The agent's execution plan for complex tasks.
    Plan(Plan),
    /// Available commands are ready or have changed
    AvailableCommandsUpdate(AvailableCommandsUpdate),
    /// The current mode of the session has changed
    CurrentModeUpdate(CurrentModeUpdate),
    /// Session configuration options have been updated.
    ConfigOptionUpdate(ConfigOptionUpdate),
    /// Session metadata has been updated (title, timestamps, custom metadata)
    SessionInfoUpdate(SessionInfoUpdate),
    /// Context window and cost update for the session.
    UsageUpdate(UsageUpdate),
}

/// The current mode of the session has changed
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct CurrentModeUpdate {
    /// The ID of the current mode
    pub current_mode_id: SessionModeId,
}

impl CurrentModeUpdate {
    /// Builds [`CurrentModeUpdate`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(current_mode_id: impl Into<SessionModeId>) -> Self {
        Self {
            current_mode_id: current_mode_id.into(),
        }
    }
}

/// Session configuration options have been updated.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct ConfigOptionUpdate {
    /// The full set of configuration options and their current values.
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub config_options: Vec<SessionConfigOption>,
}

impl ConfigOptionUpdate {
    /// Builds [`ConfigOptionUpdate`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(config_options: Vec<SessionConfigOption>) -> Self {
        Self { config_options }
    }
}

/// Update to session metadata. All fields are optional to support partial updates.
///
/// Agents send this notification to update session information like title or custom metadata.
/// This allows clients to display dynamic session names and track session state changes.
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct SessionInfoUpdate {
    /// Human-readable title for the session. Set to null to clear.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "MaybeUndefined::is_undefined")]
    #[ts(type = "string | null")]
    pub title: MaybeUndefined<String>,
    /// ISO 8601 timestamp of last activity. Set to null to clear.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "MaybeUndefined::is_undefined")]
    #[ts(type = "string | null")]
    pub updated_at: MaybeUndefined<String>,
}

impl SessionInfoUpdate {
    /// Builds [`SessionInfoUpdate`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Human-readable title for the session. Set to null to clear.
    #[must_use]
    pub fn title(mut self, title: impl IntoMaybeUndefined<String>) -> Self {
        self.title = title.into_maybe_undefined();
        self
    }

    /// ISO 8601 timestamp of last activity. Set to null to clear.
    #[must_use]
    pub fn updated_at(mut self, updated_at: impl IntoMaybeUndefined<String>) -> Self {
        self.updated_at = updated_at.into_maybe_undefined();
        self
    }
}

/// Context window and cost update for a session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct UsageUpdate {
    /// Tokens currently in context.
    pub used: u64,
    /// Total context window size in tokens.
    pub size: u64,
    /// Cumulative session cost (optional).
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub cost: Option<Cost>,
}

impl UsageUpdate {
    /// Builds [`UsageUpdate`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(used: u64, size: u64) -> Self {
        Self {
            used,
            size,
            cost: None,
        }
    }

    /// Cumulative session cost (optional).
    #[must_use]
    pub fn cost(mut self, cost: impl IntoOption<Cost>) -> Self {
        self.cost = cost.into_option();
        self
    }
}

/// Cost information for a session.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    /// Total cumulative cost for session.
    pub amount: f64,
    /// ISO 4217 currency code (e.g., "USD", "EUR").
    pub currency: String,
}

impl Cost {
    /// Builds [`Cost`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(amount: f64, currency: impl Into<String>) -> Self {
        Self {
            amount,
            currency: currency.into(),
        }
    }
}

/// A streamed item of content
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct ContentChunk {
    /// A single item of content
    pub content: ContentBlock,
    /// A unique identifier for the message this chunk belongs to.
    ///
    /// All chunks belonging to the same message share the same `messageId`.
    /// A change in `messageId` indicates a new message has started.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub message_id: Option<MessageId>,
}

impl ContentChunk {
    /// Builds [`ContentChunk`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(content: ContentBlock) -> Self {
        Self {
            content,
            message_id: None,
        }
    }

    /// A unique identifier for the message this chunk belongs to.
    ///
    /// All chunks belonging to the same message share the same `messageId`.
    /// A change in `messageId` indicates a new message has started.
    #[must_use]
    pub fn message_id(mut self, message_id: impl IntoOption<MessageId>) -> Self {
        self.message_id = message_id.into_option();
        self
    }
}

/// Available commands are ready or have changed
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/session.ts")]
#[serde(rename_all = "camelCase")]
pub struct AvailableCommandsUpdate {
    /// Commands the agent can execute
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub available_commands: Vec<AvailableCommand>,
}

impl AvailableCommandsUpdate {
    /// Builds [`AvailableCommandsUpdate`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(available_commands: Vec<AvailableCommand>) -> Self {
        Self { available_commands }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    MessageId::export(config)?;
    Cost::export(config)?;
    ContentChunk::export(config)?;
    CurrentModeUpdate::export(config)?;
    ConfigOptionUpdate::export(config)?;
    SessionInfoUpdate::export(config)?;
    UsageUpdate::export(config)?;
    AvailableCommandsUpdate::export(config)?;
    SessionUpdate::export(config)?;
    SessionInfo::export(config)?;
    NewSessionRequest::export(config)?;
    NewSessionResponse::export(config)?;
    LoadSessionRequest::export(config)?;
    LoadSessionResponse::export(config)?;
    ResumeSessionRequest::export(config)?;
    ResumeSessionResponse::export(config)?;
    CloseSessionRequest::export(config)?;
    CloseSessionResponse::export(config)?;
    ListSessionsRequest::export(config)?;
    ListSessionsResponse::export(config)?;
    DeleteSessionRequest::export(config)?;
    DeleteSessionResponse::export(config)?;
    Ok(())
}

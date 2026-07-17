use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::ContentBlock;

/// Categorizes a tool call for client presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/tool_calls.ts")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Other,
}

impl Default for ToolKind {
    /// Uses the protocol-defined fallback when a tool has no specialized category.
    fn default() -> Self {
        Self::Other
    }
}

/// Describes the execution lifecycle of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/tool_calls.ts")]
pub enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl Default for ToolCallStatus {
    /// Uses the initial lifecycle state when a tool call omits its status.
    fn default() -> Self {
        Self::Pending
    }
}

/// Wraps a standard content block produced by a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct ContentToolCallContent {
    pub content: ContentBlock,
}

/// Describes a file modification produced by a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields = nullable)]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct DiffToolCallContent {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    pub new_text: String,
}

/// References a live terminal produced by a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct TerminalToolCallContent {
    pub terminal_id: String,
}

/// Represents content that a tool call can expose to the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "acp/tool_calls.ts")]
pub enum ToolCallContent {
    Content(ContentToolCallContent),
    Diff(DiffToolCallContent),
    Terminal(TerminalToolCallContent),
}

/// Points to a file location affected by a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields = nullable)]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct ToolCallLocation {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Announces a new tool call and its initial presentation state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields = nullable)]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct ToolCall {
    pub tool_call_id: String,
    pub title: String,
    #[serde(default)]
    pub kind: ToolKind,
    #[serde(default)]
    pub status: ToolCallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ToolCallContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<ToolCallLocation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub raw_input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub raw_output: Option<Value>,
}

/// Carries a partial update to an existing tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields = nullable)]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ToolKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolCallStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ToolCallContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<ToolCallLocation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub raw_input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub raw_output: Option<Value>,
}

/// Hints how a permission choice should be presented and remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/tool_calls.ts")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// Defines one user-selectable permission choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionOptionKind,
}

/// Carries the internal parameters of `session/request_permission`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct SessionRequestPermissionRequest {
    pub session_id: String,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
}

/// Models the two legal permission outcomes without optional-field ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "acp/tool_calls.ts")]
pub enum ToolPermissionOutcome {
    Cancelled,
    Selected { option_id: String },
}

/// Carries the internal result of `session/request_permission`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/tool_calls.ts")]
pub struct SessionRequestPermissionResponse {
    pub outcome: ToolPermissionOutcome,
}

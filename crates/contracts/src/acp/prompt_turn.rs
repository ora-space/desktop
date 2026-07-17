use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{ContentBlock, ToolCall, ToolCallUpdate};

/// Carries the internal parameters of `session/prompt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct SessionPromptRequest {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

/// Explains why an agent stopped processing a prompt turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

/// Carries the internal result of `session/prompt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct SessionPromptResponse {
    pub stop_reason: StopReason,
}

/// Carries the internal parameters of the `session/cancel` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct SessionCancelNotification {
    pub session_id: String,
}

/// Describes the urgency of a plan entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub enum PlanEntryPriority {
    High,
    Medium,
    Low,
}

/// Describes the lifecycle of a plan entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub enum PlanEntryStatus {
    Pending,
    InProgress,
    Completed,
}

/// Represents one item in an agent plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct PlanEntry {
    pub content: String,
    pub priority: PlanEntryPriority,
    pub status: PlanEntryStatus,
}

/// Replaces the current agent plan shown by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct PlanUpdate {
    pub entries: Vec<PlanEntry>,
}

/// Streams one chunk of an agent-authored message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct AgentMessageChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub content: ContentBlock,
}

/// Reports cumulative monetary cost for a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

/// Reports current context usage and optional cumulative cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct UsageUpdate {
    #[ts(type = "number")]
    pub used: u64,
    #[ts(type = "number")]
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
}

/// Represents the session updates shown across the prompt-turn and tool-call flows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub enum SessionUpdate {
    Plan(PlanUpdate),
    AgentMessageChunk(AgentMessageChunk),
    ToolCall(ToolCall),
    ToolCallUpdate(ToolCallUpdate),
    UsageUpdate(UsageUpdate),
}

/// Carries the internal parameters of a `session/update` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/prompt_turn.ts")]
pub struct SessionUpdateNotification {
    pub session_id: String,
    pub update: SessionUpdate,
}

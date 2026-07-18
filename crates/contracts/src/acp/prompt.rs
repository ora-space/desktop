use super::common::SessionId;
use super::content::ContentBlock;

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none};
use ts_rs::TS;

/// Request parameters for sending a user prompt to the agent.
///
/// Contains the user's message and any additional context.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export_to = "acp/prompt.ts")]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    /// The ID of the session to send this user message to
    pub session_id: SessionId,
    /// The blocks of content that compose the user's message.
    ///
    /// As a baseline, the Agent MUST support [`ContentBlock::Text`] and [`ContentBlock::ResourceLink`],
    /// while other variants are optionally enabled via [`PromptCapabilities`].
    ///
    /// The Client MUST adapt its interface according to [`PromptCapabilities`].
    ///
    /// The client MAY include referenced pieces of context as either
    /// [`ContentBlock::Resource`] or [`ContentBlock::ResourceLink`].
    ///
    /// When available, [`ContentBlock::Resource`] is preferred
    /// as it avoids extra round-trips and allows the message to include
    /// pieces of context from sources the agent may not have access to.
    pub prompt: Vec<ContentBlock>,
}

impl PromptRequest {
    /// Builds [`PromptRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, prompt: Vec<ContentBlock>) -> Self {
        Self {
            session_id: session_id.into(),
            prompt,
        }
    }
}

/// Response from processing a user prompt.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/prompt.ts")]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    /// Indicates why the agent stopped processing the turn.
    pub stop_reason: StopReason,
}

impl PromptResponse {
    /// Builds [`PromptResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(stop_reason: StopReason) -> Self {
        Self { stop_reason }
    }
}

/// Reasons why an agent stops processing a prompt turn.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/prompt.ts")]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The turn ended successfully.
    EndTurn,
    /// The turn ended because the agent reached the maximum number of tokens.
    MaxTokens,
    /// The turn ended because the agent reached the maximum number of allowed
    /// agent requests between user turns.
    MaxTurnRequests,
    /// The turn ended because the agent refused to continue. The user prompt
    /// and everything that comes after it won't be included in the next
    /// prompt, so this should be reflected in the UI.
    Refusal,
    /// The turn was cancelled by the client via `session/cancel`.
    ///
    /// This stop reason MUST be returned when the client sends a `session/cancel`
    /// notification, even if the cancellation causes exceptions in underlying operations.
    /// Agents should catch these exceptions and return this semantically meaningful
    /// response to confirm successful cancellation.
    Cancelled,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PromptRequest::export(config)?;
    PromptResponse::export(config)?;
    StopReason::export(config)?;
    Ok(())
}

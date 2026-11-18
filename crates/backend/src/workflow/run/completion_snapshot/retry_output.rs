//! Evidence belonging to the final send of a retried interactive prompt.

use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, ToolCallStatus};
use ora_history::{HistoryRecord, SessionHistory, child_source_id};

/// Returns the final retry boundary in this turn, if the recorder observed a resend.
pub(super) fn last_retry_seq(
    history: &SessionHistory,
    user_seq: u32,
    ended_seq: u32,
) -> Option<u32> {
    history
        .lines
        .iter()
        .rev()
        .find(|line| {
            line.seq > user_seq
                && line.seq <= ended_seq
                && matches!(line.record, HistoryRecord::PromptRetried)
        })
        .map(|line| line.seq)
}

/// A provider-reported parent tool result or non-text assistant message can be the deliverable.
pub(super) fn has_non_text_result(history: &SessionHistory, start: u32, end: u32) -> bool {
    history
        .lines
        .iter()
        .filter(|line| line.seq > start && line.seq <= end)
        .any(|line| {
            let HistoryRecord::Update { update, .. } = &line.record else {
                return false;
            };
            match update.as_ref() {
                SessionUpdate::ToolCall(call) => {
                    call.status == ToolCallStatus::Completed
                        && child_source_id(call.meta.as_ref()).is_none()
                }
                SessionUpdate::AgentMessageChunk(chunk) => {
                    !matches!(chunk.content, ContentBlock::Text(_))
                        && child_source_id(chunk.meta.as_ref()).is_none()
                }
                _ => false,
            }
        })
}

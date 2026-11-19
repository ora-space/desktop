//! Evidence belonging to the final send of a retried interactive prompt.

use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, ToolCallStatus};
use ora_history::{HistoryRecord, SessionHistory, child_source_id};
use std::collections::HashSet;

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
pub(super) fn has_non_text_result(
    history: &SessionHistory,
    user_seq: u32,
    retry_seq: u32,
    end: u32,
) -> bool {
    // An ACP delta carries no attempt ID. After reopening a tool ID, a completion could
    // still belong to an unfinished call from a discarded send, including a child call.
    let unfinished_tool_ids: HashSet<_> = history
        .lines
        .iter()
        .filter(|line| line.seq > user_seq && line.seq < retry_seq)
        .filter_map(|line| match &line.record {
            HistoryRecord::Update { update, .. } => match update.as_ref() {
                SessionUpdate::ToolCall(call)
                    if matches!(
                        call.status,
                        ToolCallStatus::Pending | ToolCallStatus::InProgress
                    ) =>
                {
                    Some(&call.tool_call_id)
                }
                _ => None,
            },
            _ => None,
        })
        .collect();

    history
        .lines
        .iter()
        .filter(|line| line.seq > retry_seq && line.seq <= end)
        .any(|line| {
            let HistoryRecord::Update { update, .. } = &line.record else {
                return false;
            };
            match update.as_ref() {
                SessionUpdate::ToolCall(call) => {
                    call.status == ToolCallStatus::Completed
                        && child_source_id(call.meta.as_ref()).is_none()
                        && !unfinished_tool_ids.contains(&call.tool_call_id)
                }
                SessionUpdate::AgentMessageChunk(chunk) => {
                    !matches!(chunk.content, ContentBlock::Text(_))
                        && child_source_id(chunk.meta.as_ref()).is_none()
                }
                _ => false,
            }
        })
}

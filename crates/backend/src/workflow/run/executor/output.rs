use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::MessageId;
use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::ToolCallId;
use std::collections::HashSet;

/// Accumulates only the final assistant deliverable produced by one prompt turn.
///
/// A turn can contain explanation text, tool calls, then a final answer; the node's output is the
/// final assistant message, not the concatenation of every text run. A changed `message_id` starts
/// a fresh message, and so does a position-claiming item (a new tool, the first plan, or non-text
/// content) interrupting a run that carries no `message_id`, mirroring the assembler's contiguity
/// rules so the automatic path matches the interactive path.
#[derive(Debug, Default)]
pub struct AssistantOutputAccumulator {
    message_id: Option<MessageId>,
    text: String,
    /// A position-claiming item interrupted the current implicit, no-`messageId` text run; the next
    /// no-id text starts a fresh final message.
    interrupted: bool,
    /// Tool ids already seen this turn, so an update to a known tool claims no new position.
    seen_tool_ids: HashSet<ToolCallId>,
    /// Whether a plan has already been seen, so a plan replacement claims no new position.
    has_plan: bool,
}

impl AssistantOutputAccumulator {
    /// Records assistant text while leaving the session history responsible for full conversation.
    pub fn consume(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let Some(text) = chunk_text(&chunk.content) else {
                    // Non-text content claims its own position, so it interrupts an implicit run.
                    self.interrupted = true;
                    return;
                };
                let id_changed = self.message_id.as_ref() != chunk.message_id.as_ref();
                let implicit_interrupted = chunk.message_id.is_none() && self.interrupted;
                if id_changed || implicit_interrupted {
                    self.message_id = chunk.message_id.clone();
                    self.text.clear();
                }
                self.interrupted = false;
                self.text.push_str(text);
            }
            // A tool opening, or the first update of an unknown tool, claims a new position and
            // interrupts an implicit run; an update to a known tool does not.
            SessionUpdate::ToolCall(call) => {
                self.interrupted |= self.seen_tool_ids.insert(call.tool_call_id.clone());
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.interrupted |= self.seen_tool_ids.insert(update.tool_call_id.clone());
            }
            // Only the first plan claims a position; a replacement does not interrupt.
            SessionUpdate::Plan(_) => {
                self.interrupted |= !self.has_plan;
                self.has_plan = true;
            }
            _ => {}
        }
    }

    /// Returns the assistant's scalar output, or `None` when no assistant text was produced.
    pub fn into_output(self) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        Some(self.text)
    }
}

/// Extracts the text payload from a content block, ignoring non-text blocks.
fn chunk_text(block: &ContentBlock) -> Option<&str> {
    match block {
        ContentBlock::Text(text) => Some(&text.text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::AssistantOutputAccumulator;
    use agent_client_protocol_schema::v1::{
        ContentBlock, ContentChunk, MessageId, SessionUpdate, TextContent, ToolCall, ToolCallId,
        ToolCallUpdate, ToolCallUpdateFields,
    };
    use pretty_assertions::assert_eq;

    fn agent_text_update(message_id: Option<&str>, text: &str) -> SessionUpdate {
        let mut chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text.to_string())));
        chunk.message_id = message_id.map(MessageId::new);
        SessionUpdate::AgentMessageChunk(chunk)
    }

    /// A turn that emits explanation then a final answer keeps only the final message, matching the
    /// interactive completion path that reads the last settled assistant message.
    #[test]
    fn assistant_output_accumulator_keeps_only_the_final_message() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&agent_text_update(Some("msg-1"), "let me think "));
        accumulator.consume(&agent_text_update(Some("msg-2"), "final answer"));
        assert_eq!(accumulator.into_output(), Some("final answer".to_string()));
    }

    /// A no-`messageId` text run interrupted by a tool call starts a fresh final message, matching
    /// the assembler's contiguity rule.
    #[test]
    fn assistant_output_accumulator_keeps_only_the_final_implicit_message() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&agent_text_update(None, "let me think "));
        accumulator.consume(&SessionUpdate::ToolCall(ToolCall::new(
            ToolCallId::new("t1"),
            "look up",
        )));
        accumulator.consume(&agent_text_update(None, "final answer"));
        assert_eq!(accumulator.into_output(), Some("final answer".to_string()));
    }

    /// An update to an already-known tool claims no new position, so a no-`messageId` text run on
    /// either side of it stays one message, matching the assembler.
    #[test]
    fn assistant_output_accumulator_does_not_break_on_a_known_tool_update() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&agent_text_update(None, "foo "));
        accumulator.consume(&SessionUpdate::ToolCall(ToolCall::new(
            ToolCallId::new("t1"),
            "read",
        )));
        accumulator.consume(&agent_text_update(None, "bar "));
        accumulator.consume(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            ToolCallId::new("t1"),
            ToolCallUpdateFields::new(),
        )));
        accumulator.consume(&agent_text_update(None, "baz"));
        assert_eq!(accumulator.into_output(), Some("bar baz".to_string()));
    }

    /// User text is ignored so a mixed turn still yields only the assistant deliverable.
    #[test]
    fn assistant_output_accumulator_keeps_only_assistant_text() {
        let mut accumulator = AssistantOutputAccumulator::default();
        accumulator.consume(&SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("ignored")),
        )));
        accumulator.consume(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("hello ")),
        )));
        accumulator.consume(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("world")),
        )));
        assert_eq!(accumulator.into_output(), Some("hello world".to_string()));
    }
}

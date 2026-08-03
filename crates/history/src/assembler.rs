use crate::record::HistoryRecord;
use ora_contracts::acp::content::{ContentBlock, TextContent};
use ora_contracts::acp::plan::Plan;
use ora_contracts::acp::prompt::StopReason;
use ora_contracts::acp::session::{ContentChunk, MessageId, SessionUpdate};
use ora_contracts::acp::tool_call::{ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate};

/// One assembled record together with the position it occupies in the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledRecord {
    pub seq: u32,
    pub record: HistoryRecord,
}

/// Distinguishes the two chunk streams that share identical merging rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextKind {
    Message,
    Thought,
}

/// A message or thought still accumulating chunks.
struct PendingText {
    seq: u32,
    kind: TextKind,
    message_id: Option<MessageId>,
    text: String,
}

/// A tool call whose snapshot may still change.
///
/// `written` records that the call already reached the file under `seq`, which is
/// what turns a later change into a replacement rather than a duplicate.
struct PendingTool {
    seq: u32,
    call: ToolCall,
    written: bool,
}

/// Turns one session's live ACP update stream into settled history records.
///
/// The assembler is session-scoped rather than turn-scoped because `seq` numbers
/// positions across the whole file, and it is deliberately free of IO so the
/// merging rules can be tested against a plain list of updates.
///
/// Records are emitted as soon as an item can no longer change, which keeps a
/// crash from costing more than the item in flight. A tool call that settles
/// after a later message is therefore written out of order, which is what `seq`
/// exists to repair. A tool call that changes again after settling is re-emitted
/// under its original `seq`, so readers resolve duplicates by keeping the last.
///
/// Open items are held in per-kind collections rather than one ordered list:
/// order is carried entirely by `seq`, so nothing depends on their arrangement
/// here, and the lookups each kind needs stay direct.
pub struct HistoryAssembler {
    next_seq: u32,
    texts: Vec<PendingText>,
    tools: Vec<PendingTool>,
    plan: Option<(u32, Plan)>,
}

impl HistoryAssembler {
    /// Resumes numbering after the highest position already present in the file.
    pub fn new(next_seq: u32) -> Self {
        Self {
            next_seq,
            texts: Vec::new(),
            tools: Vec::new(),
            plan: None,
        }
    }

    /// Returns the position the next appended record will occupy.
    pub fn next_seq(&self) -> u32 {
        self.next_seq
    }

    /// Claims one position for a record the caller builds itself.
    ///
    /// Session metadata, agent switches, and gaps are not derived from the update
    /// stream, but they still occupy a place in the timeline.
    pub fn reserve_seq(&mut self) -> u32 {
        self.take_seq()
    }

    /// Records the user turn from the blocks Ora chose to keep.
    ///
    /// The provider echoes the prompt back as `user_message_chunk`, but that echo
    /// carries whatever was actually sent, including context Ora injected and
    /// deliberately excluded from history. The recorded blocks are the truth.
    pub fn push_user_prompt(&mut self, blocks: &[ContentBlock]) -> Vec<AssembledRecord> {
        blocks
            .iter()
            .map(|block| {
                let seq = self.take_seq();
                AssembledRecord {
                    seq,
                    record: HistoryRecord::Update {
                        update: SessionUpdate::UserMessageChunk(ContentChunk::new(block.clone())),
                    },
                }
            })
            .collect()
    }

    /// Applies one live provider update and returns whatever settled because of it.
    pub fn push_update(&mut self, update: &SessionUpdate) -> Vec<AssembledRecord> {
        match update {
            // The prompt was already recorded from the request, so the echo would
            // duplicate the user turn.
            SessionUpdate::UserMessageChunk(_) => Vec::new(),
            SessionUpdate::AgentMessageChunk(chunk) => self.push_text(TextKind::Message, chunk),
            SessionUpdate::AgentThoughtChunk(chunk) => self.push_text(TextKind::Thought, chunk),
            SessionUpdate::ToolCall(call) => self.upsert_tool(call.clone()),
            SessionUpdate::ToolCallUpdate(update) => self.update_tool(update),
            SessionUpdate::Plan(plan) => {
                self.replace_plan(plan.clone());
                Vec::new()
            }
            // Session chrome. The agent re-establishes it on every binding, so
            // persisting it would only preserve a copy that goes stale.
            SessionUpdate::AvailableCommandsUpdate(_)
            | SessionUpdate::CurrentModeUpdate(_)
            | SessionUpdate::ConfigOptionUpdate(_)
            | SessionUpdate::SessionInfoUpdate(_)
            | SessionUpdate::UsageUpdate(_) => Vec::new(),
        }
    }

    /// Flushes every item still open and closes the turn with its stop reason.
    pub fn end_turn(&mut self, stop_reason: StopReason) -> Vec<AssembledRecord> {
        let mut records: Vec<AssembledRecord> = std::mem::take(&mut self.texts)
            .into_iter()
            .map(PendingText::into_record)
            .chain(
                std::mem::take(&mut self.tools)
                    .into_iter()
                    .filter(|tool| !tool.written)
                    .map(|tool| tool_record(tool.seq, &tool.call)),
            )
            .chain(self.plan.take().map(|(seq, plan)| AssembledRecord {
                seq,
                record: HistoryRecord::Update {
                    update: SessionUpdate::Plan(plan),
                },
            }))
            .collect();
        records.sort_by_key(|record| record.seq);
        let seq = self.take_seq();
        records.push(AssembledRecord {
            seq,
            record: HistoryRecord::TurnEnded { stop_reason },
        });
        records
    }

    /// Merges one text chunk into its message, or settles content that cannot merge.
    fn push_text(&mut self, kind: TextKind, chunk: &ContentChunk) -> Vec<AssembledRecord> {
        let ContentBlock::Text(text) = &chunk.content else {
            // Images, resources, and links have no text to append to and no
            // identity of their own, so each one stands alone where it arrived.
            let seq = self.take_seq();
            return vec![AssembledRecord {
                seq,
                record: chunk_record(kind, chunk.message_id.clone(), chunk.content.clone()),
            }];
        };
        let existing = self.texts.iter_mut().find(|pending| {
            pending.kind == kind && pending.message_id.as_ref() == chunk.message_id.as_ref()
        });
        if let Some(pending) = existing {
            pending.text.push_str(&text.text);
            return Vec::new();
        }
        // ACP defines a changed `messageId` as a new message, so the previous one
        // on this stream can no longer grow. Chunks without an id cannot be told
        // apart and stay open until the turn ends.
        let records = match chunk.message_id {
            Some(_) => self.settle_identified_text(kind),
            None => Vec::new(),
        };
        let seq = self.take_seq();
        self.texts.push(PendingText {
            seq,
            kind,
            message_id: chunk.message_id.clone(),
            text: text.text.clone(),
        });
        records
    }

    /// Closes the open identified message on one stream before a new one starts.
    fn settle_identified_text(&mut self, kind: TextKind) -> Vec<AssembledRecord> {
        let mut settled = Vec::new();
        let mut remaining = Vec::with_capacity(self.texts.len());
        for pending in std::mem::take(&mut self.texts) {
            if pending.kind == kind && pending.message_id.is_some() {
                settled.push(pending.into_record());
            } else {
                remaining.push(pending);
            }
        }
        self.texts = remaining;
        settled
    }

    /// Installs a tool call's opening snapshot, replacing an earlier one in place.
    fn upsert_tool(&mut self, call: ToolCall) -> Vec<AssembledRecord> {
        match self.tool_index(&call.tool_call_id) {
            Some(index) => {
                let pending = &mut self.tools[index];
                pending.call = call;
                pending.settle()
            }
            None => self.open_tool(call),
        }
    }

    /// Applies one partial tool update to the snapshot it belongs to.
    fn update_tool(&mut self, update: &ToolCallUpdate) -> Vec<AssembledRecord> {
        if let Some(index) = self.tool_index(&update.tool_call_id) {
            let pending = &mut self.tools[index];
            pending.call.update(update.fields.clone());
            return pending.settle();
        }
        // An update for a call Ora never saw start still belongs to the timeline;
        // ACP only guarantees the title on the opening notification.
        let mut call = ToolCall::new(update.tool_call_id.clone(), "Tool call");
        call.update(update.fields.clone());
        self.open_tool(call)
    }

    /// Starts tracking one tool call and writes it immediately if it arrived settled.
    fn open_tool(&mut self, call: ToolCall) -> Vec<AssembledRecord> {
        let seq = self.take_seq();
        let mut pending = PendingTool {
            seq,
            call,
            written: false,
        };
        let records = pending.settle();
        self.tools.push(pending);
        records
    }

    /// Keeps only the newest plan snapshot, which is the one the turn ends with.
    fn replace_plan(&mut self, plan: Plan) {
        match &mut self.plan {
            Some((_, pending)) => *pending = plan,
            None => {
                let seq = self.take_seq();
                self.plan = Some((seq, plan));
            }
        }
    }

    fn tool_index(&self, tool_call_id: &ToolCallId) -> Option<usize> {
        self.tools
            .iter()
            .position(|pending| pending.call.tool_call_id == *tool_call_id)
    }

    fn take_seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        seq
    }
}

impl PendingTool {
    /// Emits this call when it has reached, or already passed, a terminal status.
    ///
    /// Re-emitting an already written call is deliberate: the file would otherwise
    /// keep a snapshot the conversation has moved past, and readers resolve the
    /// duplicate by keeping the last record for the position.
    fn settle(&mut self) -> Vec<AssembledRecord> {
        let terminal = matches!(
            self.call.status,
            ToolCallStatus::Completed | ToolCallStatus::Failed
        );
        if !terminal && !self.written {
            return Vec::new();
        }
        self.written = true;
        vec![tool_record(self.seq, &self.call)]
    }
}

impl PendingText {
    /// Converts one accumulated message or thought into its final record.
    fn into_record(self) -> AssembledRecord {
        AssembledRecord {
            seq: self.seq,
            record: chunk_record(
                self.kind,
                self.message_id,
                ContentBlock::Text(TextContent::new(self.text)),
            ),
        }
    }
}

/// Builds the record that carries one tool call's current snapshot.
fn tool_record(seq: u32, call: &ToolCall) -> AssembledRecord {
    AssembledRecord {
        seq,
        record: HistoryRecord::Update {
            update: SessionUpdate::ToolCall(call.clone()),
        },
    }
}

/// Rebuilds one content chunk update on the stream it belongs to.
fn chunk_record(
    kind: TextKind,
    message_id: Option<MessageId>,
    content: ContentBlock,
) -> HistoryRecord {
    let mut chunk = ContentChunk::new(content);
    chunk.message_id = message_id;
    HistoryRecord::Update {
        update: match kind {
            TextKind::Message => SessionUpdate::AgentMessageChunk(chunk),
            TextKind::Thought => SessionUpdate::AgentThoughtChunk(chunk),
        },
    }
}

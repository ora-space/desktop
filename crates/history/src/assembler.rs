use crate::record::HistoryRecord;
use agent_client_protocol_schema::v1::Plan;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use agent_client_protocol_schema::v1::{ContentChunk, MessageId, SessionUpdate};
use agent_client_protocol_schema::v1::{ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate};

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
                        update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
                            block.clone(),
                        ))),
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
            SessionUpdate::Plan(plan) => self.replace_plan(plan.clone()),
            // Session chrome. The agent re-establishes it on every binding, so
            // persisting it would only preserve a copy that goes stale.
            SessionUpdate::AvailableCommandsUpdate(_)
            | SessionUpdate::CurrentModeUpdate(_)
            | SessionUpdate::ConfigOptionUpdate(_)
            | SessionUpdate::SessionInfoUpdate(_)
            | SessionUpdate::UsageUpdate(_) => Vec::new(),
            // The official schema is non-exhaustive so a newer additive update does not
            // prevent Ora from preserving the rest of the session history.
            _ => Vec::new(),
        }
    }

    /// Flushes every item still open and closes the turn with its stop reason.
    ///
    /// Tool calls the provider never reported finishing are settled here rather
    /// than written mid-flight, because the turn ending is proof they are no
    /// longer running.
    pub fn end_turn(&mut self, stop_reason: StopReason) -> Vec<AssembledRecord> {
        let mut records: Vec<AssembledRecord> = std::mem::take(&mut self.texts)
            .into_iter()
            .map(PendingText::into_record)
            .chain(
                std::mem::take(&mut self.tools)
                    .into_iter()
                    .filter_map(|tool| tool.settle_at_turn_end(stop_reason)),
            )
            .chain(self.plan.take().map(|(seq, plan)| AssembledRecord {
                seq,
                record: HistoryRecord::Update {
                    update: Box::new(SessionUpdate::Plan(plan)),
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
            let mut records = self.interrupt_unidentified_text();
            let seq = self.take_seq();
            records.push(AssembledRecord {
                seq,
                record: chunk_record(kind, chunk.message_id.clone(), chunk.content.clone()),
            });
            return records;
        };
        let existing = self.texts.iter_mut().find(|pending| {
            pending.kind == kind && pending.message_id.as_ref() == chunk.message_id.as_ref()
        });
        if let Some(pending) = existing {
            pending.text.push_str(&text.text);
            return Vec::new();
        }
        // ACP defines a changed `messageId` as a new message, so the previous one
        // on this stream can no longer grow. Chunks without an id stay open until
        // something proves the run ended: another entry taking its own position,
        // or the turn closing.
        let records = match chunk.message_id {
            Some(_) => {
                self.settle_texts(|pending| pending.kind == kind && pending.message_id.is_some())
            }
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

    /// Settles the open texts a caller has proved can no longer grow.
    fn settle_texts(
        &mut self,
        can_no_longer_grow: impl Fn(&PendingText) -> bool,
    ) -> Vec<AssembledRecord> {
        let mut settled = Vec::new();
        let mut remaining = Vec::with_capacity(self.texts.len());
        for pending in std::mem::take(&mut self.texts) {
            if can_no_longer_grow(&pending) {
                settled.push(pending.into_record());
            } else {
                remaining.push(pending);
            }
        }
        self.texts = remaining;
        settled
    }

    /// Closes text runs that an entry claiming its own position has interrupted.
    ///
    /// Text carrying no `messageId` cannot be told apart from a later run on the
    /// same stream, so arriving contiguously is the only evidence that chunks
    /// belong to one message. An entry that takes its own position between two
    /// chunks is exactly the evidence they do not: the text resumed after work
    /// that has already happened. Merging across it would anchor the whole run to
    /// a position earlier than the entry it followed, which is how a summary ends
    /// up recorded ahead of the tool calls it describes.
    fn interrupt_unidentified_text(&mut self) -> Vec<AssembledRecord> {
        self.settle_texts(|pending| pending.message_id.is_none())
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
        let mut records = self.interrupt_unidentified_text();
        let seq = self.take_seq();
        let mut pending = PendingTool {
            seq,
            call,
            written: false,
        };
        records.extend(pending.settle());
        self.tools.push(pending);
        records
    }

    /// Keeps only the newest plan snapshot, which is the one the turn ends with.
    fn replace_plan(&mut self, plan: Plan) -> Vec<AssembledRecord> {
        match &mut self.plan {
            Some((_, pending)) => {
                *pending = plan;
                Vec::new()
            }
            None => {
                let records = self.interrupt_unidentified_text();
                let seq = self.take_seq();
                self.plan = Some((seq, plan));
                records
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

    /// Emits this call's final snapshot once the turn it belongs to has ended.
    ///
    /// ACP does not require a provider to report a terminal status, and one that
    /// simply moves on leaves the call frozen at `pending` or `in_progress`. That
    /// is indistinguishable from work still running, so the record would keep a
    /// state the conversation has already left behind.
    ///
    /// Only a turn the agent ended on its own terms is evidence that the call it
    /// left open ran to completion — having chosen to stop, the agent had whatever
    /// the tool produced. Every other ending cut the turn short, and a call open
    /// at that moment may have been interrupted rather than finished, so its
    /// unfinished status stands and `TurnEnded` records why. Crediting those with
    /// success would report an outcome nobody observed.
    ///
    /// A terminal status the provider did report is never reinterpreted.
    fn settle_at_turn_end(mut self, stop_reason: StopReason) -> Option<AssembledRecord> {
        let unfinished = matches!(
            self.call.status,
            ToolCallStatus::Pending | ToolCallStatus::InProgress
        );
        if unfinished && stop_reason == StopReason::EndTurn {
            self.call.status = ToolCallStatus::Completed;
            return Some(tool_record(self.seq, &self.call));
        }
        (!self.written).then(|| tool_record(self.seq, &self.call))
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
            update: Box::new(SessionUpdate::ToolCall(call.clone())),
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
        update: Box::new(match kind {
            TextKind::Message => SessionUpdate::AgentMessageChunk(chunk),
            TextKind::Thought => SessionUpdate::AgentThoughtChunk(chunk),
        }),
    }
}

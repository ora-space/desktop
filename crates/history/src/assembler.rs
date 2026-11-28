use crate::metadata::{TURN_END_SETTLED_META_KEY, child_source_id, retain_tool_metadata};
use crate::record::{HistoryRecord, ToolCallTiming, TurnEndedBy, TurnFailure};
use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::Meta;
use agent_client_protocol_schema::v1::Plan;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{ContentChunk, SessionUpdate};
use agent_client_protocol_schema::v1::{ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate};
use serde_json::Value;

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
    chunk: ContentChunk,
}

/// A tool call whose snapshot may still change.
///
/// `written` records that the call already reached the file under `seq`, which is
/// what turns a later change into a replacement rather than a duplicate.
struct PendingTool {
    seq: u32,
    call: ToolCall,
    written: bool,
    timing: Option<ToolCallTiming>,
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
    last_retry_seq: Option<u32>,
    texts: Vec<PendingText>,
    tools: Vec<PendingTool>,
    plans: Vec<(u32, Plan)>,
}

impl HistoryAssembler {
    /// Resumes numbering after the highest position already present in the file.
    pub fn new(next_seq: u32) -> Self {
        Self {
            next_seq,
            last_retry_seq: None,
            texts: Vec::new(),
            tools: Vec::new(),
            plans: Vec::new(),
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
                        tool_timing: None,
                    },
                }
            })
            .collect()
    }

    /// Applies one live provider update and returns whatever settled because of it.
    pub fn push_update(&mut self, update: &SessionUpdate) -> Vec<AssembledRecord> {
        self.push_update_inner(update, None)
    }

    /// Applies a tool update together with runtime timing captured for that call.
    pub fn push_timed_update(
        &mut self,
        update: &SessionUpdate,
        timing: ToolCallTiming,
    ) -> Vec<AssembledRecord> {
        self.push_update_inner(update, Some(timing))
    }

    /// Replaces timing for an open tool before a turn boundary flushes its final snapshot.
    pub fn update_tool_timing(&mut self, tool_call_id: &ToolCallId, timing: ToolCallTiming) {
        if let Some(index) = self.tool_index(tool_call_id) {
            self.tools[index].timing = Some(timing);
        }
    }

    /// Shares update assembly while keeping timing an explicit tool-only API at call sites.
    fn push_update_inner(
        &mut self,
        update: &SessionUpdate,
        timing: Option<ToolCallTiming>,
    ) -> Vec<AssembledRecord> {
        match update {
            // The prompt was already recorded from the request, so the echo would
            // duplicate the user turn.
            SessionUpdate::UserMessageChunk(_) => Vec::new(),
            SessionUpdate::AgentMessageChunk(chunk) => self.push_text(TextKind::Message, chunk),
            SessionUpdate::AgentThoughtChunk(chunk) => self.push_text(TextKind::Thought, chunk),
            SessionUpdate::ToolCall(call) => self.upsert_tool(call.clone(), timing),
            SessionUpdate::ToolCallUpdate(update) => self.update_tool(update, timing),
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
    /// Parent tools may settle here; projected child tools retain their reported
    /// state because the parent ending does not prove child completion.
    ///
    /// `ended_by` records a caller cancel or session end; `failure` records a failed turn. A turn
    /// the Agent ended on its own has neither, which is also what records written before this
    /// change look like.
    pub fn end_turn(
        &mut self,
        stop_reason: StopReason,
        ended_by: Option<TurnEndedBy>,
        failure: Option<TurnFailure>,
    ) -> Vec<AssembledRecord> {
        let retried = self.last_retry_seq.take().is_some();
        let mut records = self.flush_attempt(stop_reason, retried);
        self.tools.clear();
        records.push(AssembledRecord {
            seq: self.take_seq(),
            record: HistoryRecord::TurnEnded {
                stop_reason,
                ended_by,
                failure,
            },
        });
        records
    }

    /// Flushes abandoned items before the replacement can reuse message or tool identifiers.
    pub fn retry_prompt(&mut self) -> Vec<AssembledRecord> {
        let mut records = self.flush_attempt(StopReason::Cancelled, true);
        let seq = self.take_seq();
        self.last_retry_seq = Some(seq);
        records.push(AssembledRecord {
            seq,
            record: HistoryRecord::PromptRetried,
        });
        records
    }

    /// A retry needs the provider's actual tool status to distinguish recovery from an empty end.
    fn flush_attempt(
        &mut self,
        stop_reason: StopReason,
        preserve_tool_status: bool,
    ) -> Vec<AssembledRecord> {
        let mut records: Vec<AssembledRecord> = std::mem::take(&mut self.texts)
            .into_iter()
            .map(PendingText::into_record)
            .chain(
                self.tools
                    .iter_mut()
                    .filter_map(|tool| tool.settle_at_turn_end(stop_reason, preserve_tool_status)),
            )
            .chain(
                std::mem::take(&mut self.plans)
                    .into_iter()
                    .map(|(seq, plan)| AssembledRecord {
                        seq,
                        record: HistoryRecord::Update {
                            update: Box::new(SessionUpdate::Plan(plan)),
                            tool_timing: None,
                        },
                    }),
            )
            .collect();
        records.sort_by_key(|record| record.seq);
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
                record: chunk_record(kind, chunk.clone()),
            });
            return records;
        };
        let existing = self.texts.iter_mut().find(|pending| {
            pending.kind == kind
                && pending.chunk.message_id == chunk.message_id
                && pending.chunk.meta == chunk.meta
                && matches!(&pending.chunk.content, ContentBlock::Text(previous)
                    if previous.annotations == text.annotations && previous.meta == text.meta)
        });
        if let Some(pending) = existing {
            let ContentBlock::Text(previous) = &mut pending.chunk.content else {
                unreachable!("pending text always holds text content");
            };
            previous.text.push_str(&text.text);
            return Vec::new();
        }
        // ACP defines a changed `messageId` as a new message, so the previous one
        // on this stream can no longer grow. Chunks without an id stay open until
        // something proves the run ended: another entry taking its own position,
        // or the turn closing.
        let records = match chunk.message_id {
            Some(_) => self
                .settle_texts(|pending| pending.kind == kind && pending.chunk.message_id.is_some()),
            None => self
                .settle_texts(|pending| pending.kind == kind && pending.chunk.message_id.is_none()),
        };
        let seq = self.take_seq();
        self.texts.push(PendingText {
            seq,
            kind,
            chunk: chunk.clone(),
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
        self.settle_texts(|pending| pending.chunk.message_id.is_none())
    }

    /// Installs a tool call's opening snapshot, replacing an earlier one in place.
    fn upsert_tool(
        &mut self,
        mut call: ToolCall,
        timing: Option<ToolCallTiming>,
    ) -> Vec<AssembledRecord> {
        strip_turn_end_settled_marker(&mut call.meta);
        match self.tool_index(&call.tool_call_id) {
            // A full opening can reuse an abandoned ID. Bare deltas keep the old position.
            Some(index)
                if self
                    .last_retry_seq
                    .is_some_and(|seq| self.tools[index].seq < seq) =>
            {
                self.tools.swap_remove(index);
                self.open_tool(call, timing)
            }
            Some(index) => {
                let pending = &mut self.tools[index];
                pending.call = call;
                if timing.is_some() {
                    pending.timing = timing;
                }
                pending.settle()
            }
            None => self.open_tool(call, timing),
        }
    }

    /// Applies one partial tool update to the snapshot it belongs to.
    fn update_tool(
        &mut self,
        update: &ToolCallUpdate,
        timing: Option<ToolCallTiming>,
    ) -> Vec<AssembledRecord> {
        let mut incoming_meta = update.meta.clone();
        strip_turn_end_settled_marker(&mut incoming_meta);
        if let Some(index) = self.tool_index(&update.tool_call_id) {
            let pending = &mut self.tools[index];
            pending.call.update(update.fields.clone());
            retain_tool_metadata(&mut pending.call.meta, incoming_meta.as_ref());
            if update.fields.status.is_some() {
                strip_turn_end_settled_marker(&mut pending.call.meta);
            }
            if timing.is_some() {
                pending.timing = timing;
            }
            return pending.settle();
        }
        // An update for a call Ora never saw start still belongs to the timeline;
        // ACP only guarantees the title on the opening notification.
        let mut call = ToolCall::new(update.tool_call_id.clone(), "Tool call");
        call.update(update.fields.clone());
        retain_tool_metadata(&mut call.meta, incoming_meta.as_ref());
        self.open_tool(call, timing)
    }

    /// Starts tracking one tool call and writes it immediately if it arrived settled.
    fn open_tool(
        &mut self,
        call: ToolCall,
        timing: Option<ToolCallTiming>,
    ) -> Vec<AssembledRecord> {
        let mut records = self.interrupt_unidentified_text();
        let seq = self.take_seq();
        let mut pending = PendingTool {
            seq,
            call,
            written: false,
            timing,
        };
        records.extend(pending.settle());
        self.tools.push(pending);
        records
    }

    /// Keeps the newest plan per valid child id, plus one unattributed parent plan.
    fn replace_plan(&mut self, plan: Plan) -> Vec<AssembledRecord> {
        match self.plans.iter_mut().find(|(_, pending)| {
            child_source_id(pending.meta.as_ref()) == child_source_id(plan.meta.as_ref())
        }) {
            Some((_, pending)) => {
                *pending = plan;
                Vec::new()
            }
            None => {
                let records = self.interrupt_unidentified_text();
                let seq = self.take_seq();
                self.plans.push((seq, plan));
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

    /// Snapshots the records still open in the assembler, each carrying its assigned position.
    ///
    /// A load replays these alongside the durable history so the merged stream is gap-free and
    /// correctly ordered: records are appended out of position, and these are the ones not yet
    /// written to disk.
    pub fn pending_records(&self) -> Vec<AssembledRecord> {
        let mut records: Vec<AssembledRecord> = self
            .texts
            .iter()
            .map(PendingText::to_record)
            .chain(
                self.tools
                    .iter()
                    // A tool that has already been written is durable, not pending; snapshotting
                    // it again would duplicate it in the merged replay prefix.
                    .filter(|tool| !tool.written)
                    .map(|tool| tool_record(tool.seq, &tool.call, tool.timing.clone())),
            )
            .collect();
        for (seq, plan) in &self.plans {
            records.push(AssembledRecord {
                seq: *seq,
                record: HistoryRecord::Update {
                    update: Box::new(SessionUpdate::Plan(plan.clone())),
                    tool_timing: None,
                },
            });
        }
        records.sort_by_key(|record| record.seq);
        records
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
        vec![tool_record(self.seq, &self.call, self.timing.clone())]
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
    fn settle_at_turn_end(
        &mut self,
        stop_reason: StopReason,
        preserve_tool_status: bool,
    ) -> Option<AssembledRecord> {
        let unfinished = matches!(
            self.call.status,
            ToolCallStatus::Pending | ToolCallStatus::InProgress
        );
        // A projected child may outlive the parent prompt. Only its own terminal update proves
        // completion. Cleared or invalid sources follow the ordinary parent-stream rules.
        let projected = child_source_id(self.call.meta.as_ref()).is_some();
        if unfinished && !preserve_tool_status && !projected && stop_reason == StopReason::EndTurn {
            self.call.status = ToolCallStatus::Completed;
            self.call
                .meta
                .get_or_insert_with(Meta::new)
                .insert(TURN_END_SETTLED_META_KEY.to_string(), Value::Bool(true));
        } else if self.written {
            return None;
        }
        self.written = true;
        Some(tool_record(self.seq, &self.call, self.timing.clone()))
    }
}

/// Drops Ora's turn-end settlement marker from untrusted incoming tool updates and cleans empty maps.
fn strip_turn_end_settled_marker(meta: &mut Option<Meta>) {
    if let Some(map) = meta {
        map.remove(TURN_END_SETTLED_META_KEY);
        if map.is_empty() {
            *meta = None;
        }
    }
}

impl PendingText {
    /// Converts one accumulated message or thought into its final record.
    fn into_record(self) -> AssembledRecord {
        AssembledRecord {
            seq: self.seq,
            record: chunk_record(self.kind, self.chunk),
        }
    }

    /// Borrows one accumulated message or thought as a record without consuming it, for snapshotting
    /// the open items a load must replay before switching to live events.
    fn to_record(&self) -> AssembledRecord {
        AssembledRecord {
            seq: self.seq,
            record: chunk_record(self.kind, self.chunk.clone()),
        }
    }
}

/// Builds the record that carries one tool call's current snapshot.
fn tool_record(seq: u32, call: &ToolCall, tool_timing: Option<ToolCallTiming>) -> AssembledRecord {
    AssembledRecord {
        seq,
        record: HistoryRecord::Update {
            update: Box::new(SessionUpdate::ToolCall(call.clone())),
            tool_timing,
        },
    }
}

/// Rebuilds one content chunk update on the stream it belongs to.
fn chunk_record(kind: TextKind, chunk: ContentChunk) -> HistoryRecord {
    HistoryRecord::Update {
        update: Box::new(match kind {
            TextKind::Message => SessionUpdate::AgentMessageChunk(chunk),
            TextKind::Thought => SessionUpdate::AgentThoughtChunk(chunk),
        }),
        tool_timing: None,
    }
}

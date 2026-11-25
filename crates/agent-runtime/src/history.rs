use crate::host::RuntimeEvents;
use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::MessageId;
use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::StopReason;
use ora_domain::{AgentRef, HistoryState, Session, SessionId};
use ora_history::{
    AgentSwitch, AssembledRecord, HistoryAssembler, HistoryClock, HistoryError, HistoryRecord,
    HistoryWriter, SCHEMA_VERSION, SessionMeta, ToolCallTiming, TurnEndedBy, TurnFailure,
};
use ora_logging::ora_warn;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

/// Supplies history timestamps from Ora's process-wide local clock.
///
/// History files are read by people, so their timestamps follow the same local
/// timezone every other Ora surface presents.
#[derive(Clone, Copy, Debug)]
pub(super) struct LocalHistoryClock;

impl HistoryClock for LocalHistoryClock {
    fn now_local(&self) -> OffsetDateTime {
        ora_logging::clock::now_local()
    }
}

/// Reports whether an attempt to record just cost this session its history.
///
/// Only the transition matters to callers: a recorder that already stopped stays
/// silent, because the session was marked degraded when it first failed and
/// repeating that would overwrite the original reason with a later symptom.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RecordOutcome {
    Continued,
    JustFailed { reason: String },
}

/// Records one session's conversation and remembers when it stopped being able to.
///
/// Every method returns whether this call broke the history rather than a
/// `Result`, because a failed write is never something the runtime retries: the
/// session stops accepting prompts and the user is told why. Continuing to append
/// after a failure would produce a file that looks complete while missing the
/// middle of a conversation.
///
/// The clock is injected rather than reached for, so a test can assert on what a
/// recorded sequence means without the process-wide local clock the runtime
/// installs at startup. Production callers pass [`LocalHistoryClock`], which the
/// type parameter defaults to so the runtime never has to name it.
///
/// Every line is handed to the host's [`RuntimeEvents::record_settled`] right after the file
/// holds it, so a host mirroring the conversation never holds a line the file lacks.
pub(super) struct SessionRecorder<E, C: HistoryClock = LocalHistoryClock> {
    session_id: SessionId,
    writer: HistoryWriter<C>,
    assembler: HistoryAssembler,
    state: RecorderState,
    events: E,
    last_preview: Option<std::time::Instant>,
}

enum RecorderState {
    Recording,
    Stopped,
}

impl<E: RuntimeEvents, C: HistoryClock> SessionRecorder<E, C> {
    /// Opens the recorder for one session, resuming its position counter.
    ///
    /// A session whose history already failed opens stopped, so a restart does not
    /// quietly resume appending after the gap its failure left.
    pub(super) fn open(
        root: &Path,
        session_id: &str,
        next_seq: u32,
        history_state: &HistoryState,
        events: E,
        clock: C,
    ) -> Result<Self, HistoryError> {
        Ok(Self {
            session_id: SessionId::new(session_id.to_string()),
            writer: HistoryWriter::open(root, session_id, clock)?,
            assembler: HistoryAssembler::new(next_seq),
            state: match history_state {
                HistoryState::Writable => RecorderState::Recording,
                HistoryState::Degraded { .. } => RecorderState::Stopped,
            },
            events,
            last_preview: None,
        })
    }

    /// Returns the file this recorder appends to.
    pub(super) fn path(&self) -> PathBuf {
        self.writer.path().to_path_buf()
    }

    /// Returns the durable byte cutoff a load can replay up to without seeing later appends.
    pub(super) fn durable_bytes(&self) -> u64 {
        self.writer.durable_bytes()
    }

    /// Snapshots the assembler's still-open records, each carrying its assigned position.
    pub(super) fn pending_records(&self) -> Vec<AssembledRecord> {
        self.assembler.pending_records()
    }

    /// Writes the header that opens a newly created session's history.
    pub(super) fn record_meta(&mut self, session: &Session, cwd: &Path) -> RecordOutcome {
        self.append_standalone(HistoryRecord::Meta(SessionMeta {
            schema_version: SCHEMA_VERSION,
            session_id: session.id.to_string(),
            workspace_id: session.workspace_id.to_string(),
            agent_ref: session.agent_ref.clone(),
            agent_session_id: session.agent_session_id.clone(),
            cwd: cwd.to_path_buf(),
        }))
    }

    /// Records the user's turn from the blocks Ora chose to keep.
    pub(super) fn record_prompt(&mut self, prompt: &[ContentBlock]) -> RecordOutcome {
        let records = self.assembler.push_user_prompt(prompt);
        self.append(&records)
    }

    /// Records the user's turn with the host's identity for the message on every block.
    ///
    /// ACP's own `messageId` carries it, so the history schema is unchanged and a reader that
    /// does not know host turn identities simply ignores it.
    pub(super) fn record_identified_prompt(
        &mut self,
        prompt: &[ContentBlock],
        message_id: MessageId,
    ) -> RecordOutcome {
        let mut records = self.assembler.push_user_prompt(prompt);
        for record in &mut records {
            if let HistoryRecord::Update { update, .. } = &mut record.record
                && let SessionUpdate::UserMessageChunk(chunk) = update.as_mut()
            {
                chunk.message_id = Some(message_id.clone());
            }
        }
        self.append(&records)
    }

    /// Folds one streamed update in and writes whatever settled because of it.
    pub(super) fn record_update(&mut self, update: &SessionUpdate) -> RecordOutcome {
        let records = self.assembler.push_update(update);
        let outcome = self.append(&records);
        self.preview_pending();
        outcome
    }

    /// Records a tool update with timing supplied by the session-local runtime tracker.
    pub(super) fn record_timed_update(
        &mut self,
        update: &SessionUpdate,
        timing: &ora_contracts::ToolCallTiming,
    ) -> RecordOutcome {
        let records = self.assembler.push_timed_update(
            update,
            ToolCallTiming {
                started_at: timing.started_at.clone(),
                duration_ms: timing.duration_ms,
            },
        );
        let outcome = self.append(&records);
        self.preview_pending();
        outcome
    }

    /// Freezes open tool timing before the ordinary turn-boundary flush.
    pub(super) fn finish_tool_timings(
        &mut self,
        timings: impl IntoIterator<
            Item = (agent_client_protocol_schema::v1::ToolCallId, ToolCallTiming),
        >,
    ) {
        for (tool_call_id, timing) in timings {
            self.assembler.update_tool_timing(&tool_call_id, timing);
        }
    }

    /// Persists the retry boundary after the cancelled request's final updates.
    pub(super) fn record_retry(&mut self) -> RecordOutcome {
        let records = self.assembler.retry_prompt();
        self.append(&records)
    }

    /// Closes the turn, flushing every item still open, and records why a turn that did not
    /// complete ended: `ended_by` for a caller cancel or session end, `failure` for a failed turn.
    pub(super) fn record_turn_end_with(
        &mut self,
        stop_reason: StopReason,
        ended_by: Option<TurnEndedBy>,
        failure: Option<TurnFailure>,
    ) -> RecordOutcome {
        let records = self.assembler.end_turn(stop_reason, ended_by, failure);
        let outcome = self.append(&records);
        if self.events.wants_record_previews() {
            self.events.record_preview(&self.session_id, &[]);
        }
        self.last_preview = None;
        outcome
    }

    /// Snapshot assembly is capped at ten times per second; token rate cannot amplify history IO
    /// or host allocations. A turn boundary always clears the last preview without waiting.
    fn preview_pending(&mut self) {
        if !self.events.wants_record_previews() || matches!(self.state, RecorderState::Stopped) {
            return;
        }
        let now = std::time::Instant::now();
        if self
            .last_preview
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(100))
        {
            return;
        }
        let pending = self.assembler.pending_records();
        if pending.is_empty() {
            return;
        }
        self.last_preview = Some(now);
        self.events.record_preview(&self.session_id, &pending);
    }

    /// Records that the conversation moved to another agent.
    pub(super) fn record_agent_switch(
        &mut self,
        from: AgentRef,
        to: AgentRef,
        agent_session_id: String,
    ) -> RecordOutcome {
        self.append_standalone(HistoryRecord::AgentSwitched(AgentSwitch {
            from,
            to,
            agent_session_id,
        }))
    }

    /// Records that the agent bound by the last switch was given the transcript.
    ///
    /// The provider session is named so the line states which binding was
    /// brought up to date, rather than leaving that to be inferred from where
    /// the line happens to sit relative to the switch above it.
    pub(super) fn record_handoff_delivered(&mut self, agent_session_id: String) -> RecordOutcome {
        self.append_standalone(HistoryRecord::HandoffDelivered { agent_session_id })
    }

    /// Returns a stopped recorder to service by first recording what it lost.
    ///
    /// The gap is written before anything else so the conversation never contains
    /// a discontinuity that cannot be seen — including by the transcript handed to
    /// another agent later.
    pub(super) fn resume(&mut self, reason: String) -> RecordOutcome {
        self.state = RecorderState::Recording;
        self.append_standalone(HistoryRecord::Gap { reason })
    }

    /// Appends one record that the assembler does not produce from the ACP stream.
    ///
    /// Session chrome — the header, a switch, a delivery, a gap — has no streamed
    /// item to settle, so it claims a position directly instead of arriving as
    /// something the assembler folded together.
    fn append_standalone(&mut self, record: HistoryRecord) -> RecordOutcome {
        let seq = self.assembler.reserve_seq();
        self.append(&[AssembledRecord { seq, record }])
    }

    /// Appends records one line at a time, stopping this recorder for good if a write fails.
    ///
    /// Each line reaches the host before the next is written, so a host mirroring the history
    /// that crashes between the two writes is behind the file by at most one line rather than by
    /// a whole turn's worth of settled items.
    fn append(&mut self, records: &[AssembledRecord]) -> RecordOutcome {
        if let RecorderState::Stopped = self.state {
            return RecordOutcome::Continued;
        }
        for record in records {
            match self.writer.append(std::slice::from_ref(record)) {
                Ok(lines) => {
                    for line in &lines {
                        self.events.record_settled(&self.session_id, line);
                    }
                }
                Err(error) => {
                    self.state = RecorderState::Stopped;
                    let reason = describe(&error);
                    ora_warn!(
                        path = %self.writer.path().display(),
                        error = %error,
                        "session history write failed",
                    );
                    return RecordOutcome::JustFailed { reason };
                }
            }
        }
        RecordOutcome::Continued
    }
}

/// Renders one history failure as the short sentence the user has to act on.
///
/// The chain is walked to the operating system's own message because "failed to
/// append" alone tells nobody whether to free disk space or fix a permission.
fn describe(error: &HistoryError) -> String {
    let mut description = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        description.push_str(": ");
        description.push_str(&cause.to_string());
        source = cause.source();
    }
    description
}

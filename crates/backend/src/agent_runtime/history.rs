use agent_client_protocol_schema::v1::ContentBlock;
use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::StopReason;
use ora_domain::{AgentCli, HistoryState, Session};
use ora_history::{
    AgentSwitch, AssembledRecord, HistoryAssembler, HistoryClock, HistoryError, HistoryRecord,
    HistoryWriter, SCHEMA_VERSION, SessionMeta,
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
pub(super) struct SessionRecorder {
    writer: HistoryWriter<LocalHistoryClock>,
    assembler: HistoryAssembler,
    state: RecorderState,
}

enum RecorderState {
    Recording,
    Stopped,
}

impl SessionRecorder {
    /// Opens the recorder for one session, resuming its position counter.
    ///
    /// A session whose history already failed opens stopped, so a restart does not
    /// quietly resume appending after the gap its failure left.
    pub(super) fn open(
        root: &Path,
        session_id: &str,
        next_seq: u32,
        history_state: &HistoryState,
    ) -> Result<Self, HistoryError> {
        Ok(Self {
            writer: HistoryWriter::open(root, session_id, LocalHistoryClock)?,
            assembler: HistoryAssembler::new(next_seq),
            state: match history_state {
                HistoryState::Writable => RecorderState::Recording,
                HistoryState::Degraded { .. } => RecorderState::Stopped,
            },
        })
    }

    /// Returns the file this recorder appends to.
    pub(super) fn path(&self) -> PathBuf {
        self.writer.path().to_path_buf()
    }

    /// Writes the header that opens a newly created session's history.
    pub(super) fn record_meta(&mut self, session: &Session, cwd: &Path) -> RecordOutcome {
        let seq = self.assembler.reserve_seq();
        self.append(&[AssembledRecord {
            seq,
            record: HistoryRecord::Meta(SessionMeta {
                schema_version: SCHEMA_VERSION,
                session_id: session.id.to_string(),
                task_id: session.task_id.to_string(),
                agent_cli: session.agent_cli,
                agent_session_id: session.agent_session_id.clone(),
                cwd: cwd.to_path_buf(),
            }),
        }])
    }

    /// Records the user's turn from the blocks Ora chose to keep.
    pub(super) fn record_prompt(&mut self, prompt: &[ContentBlock]) -> RecordOutcome {
        let records = self.assembler.push_user_prompt(prompt);
        self.append(&records)
    }

    /// Folds one streamed update in and writes whatever settled because of it.
    pub(super) fn record_update(&mut self, update: &SessionUpdate) -> RecordOutcome {
        let records = self.assembler.push_update(update);
        self.append(&records)
    }

    /// Closes the turn, flushing every item still open.
    pub(super) fn record_turn_end(&mut self, stop_reason: StopReason) -> RecordOutcome {
        let records = self.assembler.end_turn(stop_reason);
        self.append(&records)
    }

    /// Records that the conversation moved to another CLI.
    pub(super) fn record_agent_switch(
        &mut self,
        from: AgentCli,
        to: AgentCli,
        agent_session_id: String,
    ) -> RecordOutcome {
        let seq = self.assembler.reserve_seq();
        self.append(&[AssembledRecord {
            seq,
            record: HistoryRecord::AgentSwitched(AgentSwitch {
                from,
                to,
                agent_session_id,
            }),
        }])
    }

    /// Returns a stopped recorder to service by first recording what it lost.
    ///
    /// The gap is written before anything else so the conversation never contains
    /// a discontinuity that cannot be seen — including by the transcript handed to
    /// another agent later.
    pub(super) fn resume(&mut self, reason: String) -> RecordOutcome {
        self.state = RecorderState::Recording;
        let seq = self.assembler.reserve_seq();
        self.append(&[AssembledRecord {
            seq,
            record: HistoryRecord::Gap { reason },
        }])
    }

    /// Appends a batch, stopping this recorder for good if the write fails.
    fn append(&mut self, records: &[AssembledRecord]) -> RecordOutcome {
        match self.state {
            RecorderState::Stopped => RecordOutcome::Continued,
            RecorderState::Recording => match self.writer.append(records) {
                Ok(()) => RecordOutcome::Continued,
                Err(error) => {
                    self.state = RecorderState::Stopped;
                    let reason = describe(&error);
                    ora_warn!(
                        path = %self.writer.path().display(),
                        error = %error,
                        "session history write failed",
                    );
                    RecordOutcome::JustFailed { reason }
                }
            },
        }
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

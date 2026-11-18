//! Derives the completion snapshot persisted when an interactive node parks.
//!
//! The snapshot captures the turn that just settled — its identity, final assistant text digest,
//! stop reason, and declared-output blockers — so the read projection can show the same readiness
//! that `completeWorkflowNode` later re-validates. All inputs come from the session's durable
//! history so the park path and the completion path share one source.

mod derived;
mod retry_output;
pub(crate) use derived::{
    completion_final_text, derive_completion_snapshot, trailing_assistant_text,
};

use ora_agent_runtime::PromptEndCause;
use ora_application::{AgentCompletion, AgentOutputContract};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Identifies one settled turn by the positions of its user message and its `TurnEnded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnKey {
    pub user_seq: u32,
    pub ended_seq: u32,
}

/// Which prompt opened the parked turn, so the workflow's own opening turn can stay unconfirmed.
///
/// The workflow must not treat the agent's first reply as the user accepting the step: the agent
/// may have ended that reply with a question, and an `EndTurn` only says the reply finished. Later
/// turns are opened by the user, who has already participated, so their `EndTurn` remains the
/// completion signal the completion ADR specifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnOrigin {
    /// The workflow-owned opening prompt; the user has not answered anything yet.
    Workflow,
    /// A turn opened by a human reply, or a snapshot materialized from history alone.
    Human,
}

/// Why the owning prompt stopped without a usable end turn.
///
/// The blocker kind says what the turn looked like; the stop cause says which side ended it. The
/// distinction matters because `Cancelled` and `turn_unknown` can come from the user stopping the
/// prompt, the inactivity policy giving up on it, the client dropping the stream, or the session
/// crashing — and the UI must not show a system failure as the user's own stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopCause {
    /// The user cancelled the prompt from the session UI.
    UserStopped,
    /// The frozen prompt-inactivity policy exhausted its windows.
    PromptTimedOut,
    /// The prompt stream was dropped before the turn settled.
    StreamDropped,
    /// The owning agent session crashed or ended without a stop reason.
    SessionCrashed,
    /// The end reason could not be determined at the park site.
    Unknown,
}

impl StopCause {
    /// Maps the owning prompt's end cause to the park's stop cause.
    ///
    /// A settled turn has no stop cause: its stop reason and blockers describe the end on their
    /// own (ADR D4). Every interruption keeps its own cause, including one that cannot be
    /// attributed, so a `turn_stopped` park is never silently read as the user's own stop.
    pub(crate) fn of_prompt_end(end: Option<PromptEndCause>) -> Option<Self> {
        match end {
            Some(PromptEndCause::UserStopped) => Some(Self::UserStopped),
            Some(PromptEndCause::InactivityTimeout) => Some(Self::PromptTimedOut),
            Some(PromptEndCause::StreamDropped) => Some(Self::StreamDropped),
            Some(PromptEndCause::SessionCrashed) => Some(Self::SessionCrashed),
            Some(PromptEndCause::Unknown) => Some(Self::Unknown),
            Some(PromptEndCause::Settled) | None => None,
        }
    }
}

/// How a parked node is shown: waiting for the user, or stopped by an abnormal end.
///
/// The classification is computed where the snapshot is written, because only the park site knows
/// the turn origin and the stop cause; the projection must not re-infer it from blockers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParkKind {
    /// Waiting for the user to confirm the opening turn, answer, continue, or choose how to go on.
    AwaitingUser,
    /// The turn stopped for a reason that is not the user's participation.
    Abnormal,
}

/// A stable reason that an awaiting interactive node may not complete without an explicit force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompletionBlockerKind {
    TurnStopped,
    TurnTruncated,
    TurnRefused,
    TurnUnknown,
    /// The workflow's opening turn ended normally, but the user has not confirmed the step.
    TurnUnconfirmed,
    /// The current turn has no accepted agent completion signal.
    AgentSignalPending,
    /// The current turn has an accepted signal, but no human has confirmed it yet.
    AgentConfirmationPending,
    StructuredInvalid,
    OutputMissing,
    /// No existing workspace file matched a declared `completion.files` entry.
    FileMissing,
    /// A `mustChange` declaration has existing matches but none differ from the node baseline.
    FileUnchanged,
}

/// One blocker, with params carrying implementation detail the UI can localize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompletionBlocker {
    pub kind: CompletionBlockerKind,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl CompletionBlocker {
    /// Builds a blocker without extra params.
    pub(crate) fn new(kind: CompletionBlockerKind) -> Self {
        Self {
            kind,
            params: Value::Null,
        }
    }

    /// Builds a blocker with a human-readable detail for the matching UI reason.
    fn with_message(kind: CompletionBlockerKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            params: json!({ "message": message.into() }),
        }
    }
}

/// Warning written when a declared `mustChange` cannot be judged because its baseline is missing.
pub(crate) const FILE_CHECK_UNAVAILABLE: &str = "file_check_unavailable";

/// A recovered ACP request ended without delivering any observable result.
pub(crate) const RETRY_NO_RESULT: &str = "retry_without_result";

/// Adds the baseline-missing warning to a park-derived snapshot.
///
/// The baseline's presence is a fact known where the node parks, so the warning travels with the
/// snapshot instead of being invented later from a rejected completion (ADR D4). A declaration
/// with no `mustChange` file has nothing unavailable, so it adds no noise.
pub(crate) fn with_file_check_warning(
    mut snapshot: CompletionSnapshot,
    completion: Option<&AgentCompletion>,
    baseline_available: bool,
) -> CompletionSnapshot {
    let needs_baseline =
        completion.is_some_and(|completion| completion.files.iter().any(|file| file.must_change));
    if needs_baseline
        && !baseline_available
        && !snapshot
            .warnings
            .iter()
            .any(|warning| warning == FILE_CHECK_UNAVAILABLE)
    {
        snapshot.warnings.push(FILE_CHECK_UNAVAILABLE.to_string());
    }
    snapshot
}

/// The persisted readiness record for one parked interactive turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompletionSnapshot {
    /// The last user message that opened this turn; always present when the history has one.
    pub user_seq: Option<u32>,
    /// The settled turn identity, or `None` when this turn has no `TurnEnded`.
    pub turn_key: Option<TurnKey>,
    /// Snake-case stop reason of the settled turn, or `None` when the turn did not settle.
    pub stop_reason: Option<String>,
    /// Which prompt opened the turn; absent on snapshots written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_origin: Option<TurnOrigin>,
    /// Why the owning prompt stopped, when the park site knew it; null when not applicable.
    #[serde(default)]
    pub stop_cause: Option<StopCause>,
    /// The display classification for this park; absent when the turn is ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub park_kind: Option<ParkKind>,
    pub blockers: Vec<CompletionBlocker>,
    pub warnings: Vec<String>,
    /// SHA-256 of the final assistant text; `None` when there is no settled turn.
    pub output_digest: Option<String>,
    /// True when cleanup timed out waiting for the actor and a later `prepare` may repair it.
    pub provisional: bool,
}

/// Whether the node requires an explicit agent completion signal.
///
/// `EndTurn` is the legacy policy: a normal `EndTurn` with no blockers is ready. `Agent` keeps the
/// node awaiting until the agent's completion request for the settled turn is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompletionPolicy {
    Agent,
    #[default]
    EndTurn,
}

/// The agent's accepted completion request for one settled turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSignal {
    pub signal_id: String,
    pub requested_at: i64,
    pub turn_key: TurnKey,
}

/// Who approved an accepted agent completion signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfirmationBy {
    Human,
    Auto,
}

/// The human (or configured auto-accept) approval of an accepted signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompletionConfirmation {
    pub by: ConfirmationBy,
    pub at: i64,
}

/// Completion metadata that lives beside the snapshot on the node payload.
///
/// Keeping it outside `CompletionSnapshot` lets legacy snapshots stay byte-compatible: old
/// payloads have no policy and therefore read as `EndTurn`. New parks write it through
/// [`completion_payload_with_meta`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CompletionMeta {
    pub policy: CompletionPolicy,
    pub agent_signal: Option<AgentSignal>,
    pub confirmation: Option<CompletionConfirmation>,
    /// Whether a settled human turn accepts an agent signal without waiting for the user.
    pub auto_accept: bool,
}

impl CompletionMeta {
    /// Reads the policy, signal and confirmation from a node payload, defaulting to legacy end-turn.
    pub(crate) fn from_payload(payload: Option<&str>) -> Self {
        let Some(payload) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return Self::default();
        };
        let policy = payload
            .get("completionPolicy")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let agent_signal = payload
            .get("agentSignal")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let confirmation = payload
            .get("confirmation")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let auto_accept = payload
            .get("autoAcceptAgentCompletion")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Self {
            policy,
            agent_signal,
            confirmation,
            auto_accept,
        }
    }
}

/// Wraps a snapshot and its completion metadata in the node-run payload shape.
///
/// The signal gate is materialized into `completion.blockers` here so the read projection and the
/// completion gate share one fact. `agentSignal` and `confirmation` are always written, using
/// `null` when absent: the payload is merged with `json_patch`, and omitting a key would leave a
/// previous turn's signal or confirmation behind.
pub(crate) fn completion_payload_with_meta(
    snapshot: &CompletionSnapshot,
    meta: &CompletionMeta,
) -> Value {
    let snapshot = snapshot.clone().with_agent_signal_gate(meta);
    let mut payload = completion_payload(&snapshot);
    if let Some(user_seq) = snapshot.user_seq
        && snapshot.park_kind == Some(ParkKind::Abnormal)
        && (snapshot.stop_cause == Some(StopCause::PromptTimedOut)
            || snapshot
                .warnings
                .iter()
                .any(|warning| warning == RETRY_NO_RESULT))
    {
        // The payload merge retains this audit after a subsequent healthy human turn.
        payload["promptFailures"][user_seq.to_string()] = json!({ "turnKey": snapshot.turn_key, "stopCause": snapshot.stop_cause,
            "warnings": snapshot.warnings, "blockers": snapshot.blockers });
    }
    payload["completionPolicy"] = serde_json::to_value(meta.policy).unwrap_or(Value::Null);
    payload["agentSignal"] = meta
        .agent_signal
        .as_ref()
        .and_then(|signal| serde_json::to_value(signal).ok())
        .unwrap_or(Value::Null);
    payload["confirmation"] = meta
        .confirmation
        .as_ref()
        .and_then(|confirmation| serde_json::to_value(confirmation).ok())
        .unwrap_or(Value::Null);
    payload["autoAcceptAgentCompletion"] = Value::Bool(meta.auto_accept);
    payload
}

/// Returns whether one blocker describes the turn itself rather than the agent signal gate.
pub(crate) fn has_precise_stop_blocker(blockers: &[CompletionBlocker]) -> bool {
    blockers.iter().any(|blocker| {
        matches!(
            blocker.kind,
            CompletionBlockerKind::TurnStopped
                | CompletionBlockerKind::TurnTruncated
                | CompletionBlockerKind::TurnRefused
                | CompletionBlockerKind::TurnUnknown
                | CompletionBlockerKind::StructuredInvalid
                | CompletionBlockerKind::OutputMissing
        )
    })
}

impl CompletionSnapshot {
    /// Materializes the agent policy's signal gate into the persisted blockers and park kind.
    ///
    /// A precise stop (timeout, refusal, invalid output) already decides the turn, so the signal
    /// gate is not stacked on top of it. Normal waiting for a signal or a confirmation is the
    /// user's participation and must park as `awaiting_user`, not as an abnormal stop.
    pub(crate) fn with_agent_signal_gate(mut self, meta: &CompletionMeta) -> Self {
        if meta.policy == CompletionPolicy::Agent
            && meta
                .agent_signal
                .as_ref()
                .is_some_and(|signal| Some(signal.turn_key) == self.turn_key)
        {
            let count = self.blockers.len();
            self.blockers
                .retain(|blocker| blocker.params["reason"] != RETRY_NO_RESULT);
            if count != self.blockers.len() {
                self.warnings.retain(|warning| warning != RETRY_NO_RESULT);
            }
            if count != self.blockers.len()
                && self.blockers.is_empty()
                && self.turn_origin == Some(TurnOrigin::Workflow)
            {
                self.blockers.push(CompletionBlocker::new(
                    CompletionBlockerKind::TurnUnconfirmed,
                ));
            }
        }
        if meta.policy != CompletionPolicy::Agent || has_precise_stop_blocker(&self.blockers) {
            return self;
        }
        self.blockers.retain(|blocker| {
            !matches!(
                blocker.kind,
                CompletionBlockerKind::AgentSignalPending
                    | CompletionBlockerKind::AgentConfirmationPending
            )
        });
        let signal_matches = self.turn_key.is_some()
            && meta
                .agent_signal
                .as_ref()
                .is_some_and(|signal| Some(signal.turn_key) == self.turn_key);
        if !signal_matches {
            self.blockers.push(CompletionBlocker::new(
                CompletionBlockerKind::AgentSignalPending,
            ));
        } else if meta.confirmation.is_none() {
            self.blockers.push(CompletionBlocker::new(
                CompletionBlockerKind::AgentConfirmationPending,
            ));
        }
        self.park_kind = park_kind(&self.blockers, self.provisional, self.stop_cause);
        self
    }
}

///
/// `awaiting_user` is reserved for the three cases where the turn belongs to the user's
/// participation: an unconfirmed opening turn, the user's own stop, and a provisional snapshot
/// that has not settled yet. Everything else is an abnormal stop, including a `turn_stopped` park
/// whose cause could not be determined: assuming the user stopped would show a timeout as if the
/// user had chosen it (ADR D4). A ready snapshot has no park kind at all.
fn park_kind(
    blockers: &[CompletionBlocker],
    provisional: bool,
    stop_cause: Option<StopCause>,
) -> Option<ParkKind> {
    if blockers.is_empty() {
        return None;
    }
    if provisional {
        return Some(ParkKind::AwaitingUser);
    }
    // Waiting for the user's participation includes the agent-signal gates: a normal turn without
    // a signal, or with a signal awaiting confirmation, is not an abnormal stop.
    let participation_only = blockers.iter().all(|blocker| {
        matches!(
            blocker.kind,
            CompletionBlockerKind::TurnUnconfirmed
                | CompletionBlockerKind::AgentSignalPending
                | CompletionBlockerKind::AgentConfirmationPending
        )
    });
    if participation_only {
        return Some(ParkKind::AwaitingUser);
    }
    match blockers {
        [blocker]
            if blocker.kind == CompletionBlockerKind::TurnStopped
                && stop_cause == Some(StopCause::UserStopped) =>
        {
            Some(ParkKind::AwaitingUser)
        }
        _ => Some(ParkKind::Abnormal),
    }
}

/// Wraps one snapshot in the node-run payload shape the projection parses.
pub(crate) fn completion_payload(snapshot: &CompletionSnapshot) -> Value {
    json!({ "completion": snapshot })
}

/// Builds the conservative snapshot used when the session history cannot be read at park time.
pub(crate) fn unknown_completion_snapshot(
    provisional: bool,
    origin: TurnOrigin,
    stop_cause: Option<StopCause>,
) -> CompletionSnapshot {
    let blockers = vec![CompletionBlocker::new(CompletionBlockerKind::TurnUnknown)];
    CompletionSnapshot {
        user_seq: None,
        turn_key: None,
        stop_reason: None,
        turn_origin: Some(origin),
        stop_cause,
        park_kind: park_kind(&blockers, provisional, stop_cause),
        blockers,
        warnings: vec!["history_unavailable".to_string()],
        output_digest: None,
        provisional,
    }
}

/// Builds the payload patch that records why a human forced an incomplete completion.
///
/// The audit is a payload key of its own (`completion_forced`), never mixed into the live
/// `completion` snapshot, so the projection keeps rejecting a forced node's blockers on the next
/// read while the record of the decision survives the completion. `blockers` is the effective set
/// the completion actually covered — the persisted snapshot plus anything `prepare` recomputed —
/// and holds no user-authored text. `stop_cause` separates the abnormal sources the blocker kinds
/// cannot express, such as a prompt timeout versus a user stop.
pub(crate) fn forced_completion_audit(
    snapshot: &CompletionSnapshot,
    blockers: &[CompletionBlocker],
) -> Value {
    let without_turn = snapshot.turn_key.is_none();
    let mut audit = json!({
        "completion_forced": {
            "by": "human",
            "blockers": blockers,
            "stop_cause": snapshot.stop_cause,
            "without_turn": without_turn,
            "output_source": if without_turn {
                "last_assistant_message"
            } else {
                "turn_window"
            },
        }
    });
    if snapshot
        .warnings
        .iter()
        .any(|warning| warning == RETRY_NO_RESULT)
    {
        audit["completion_forced"]["retry_reason"] = json!(RETRY_NO_RESULT);
    }
    audit
}

/// Returns the blocker the declared output contract adds for a settled turn's final text.
///
/// The park derivation and the completion-time recomputation share this rule, so a stored snapshot
/// that looks ready cannot let a completion cross an output-contract blocker the park would have
/// recorded (ADR D2, invariant 4).
pub(crate) fn output_contract_blocker(
    output_contract: Option<&AgentOutputContract>,
    final_text: Option<&str>,
) -> Option<CompletionBlocker> {
    match output_contract {
        None | Some(AgentOutputContract::None) => None,
        Some(AgentOutputContract::Text) => final_text
            .is_some_and(|text| text.trim().is_empty())
            .then(|| CompletionBlocker::new(CompletionBlockerKind::OutputMissing)),
        Some(AgentOutputContract::Structured { .. }) => {
            match super::executor::apply_output_contract(
                output_contract,
                final_text.map(str::to_string),
                "interactive-node",
            ) {
                Ok(_) => None,
                Err(error) => Some(CompletionBlocker::with_message(
                    CompletionBlockerKind::StructuredInvalid,
                    error.to_string(),
                )),
            }
        }
    }
}

/// Parses the completion snapshot stored on a node-run payload, if it is present and valid.
pub(crate) fn parse_completion_snapshot(payload: Option<&str>) -> Option<CompletionSnapshot> {
    let payload: Value = serde_json::from_str(payload?).ok()?;
    serde_json::from_value(payload.get("completion")?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        AgentSignal, CompletionBlocker, CompletionBlockerKind, CompletionConfirmation,
        CompletionMeta, CompletionPolicy, CompletionSnapshot, ConfirmationBy, ParkKind, StopCause,
        TurnKey, TurnOrigin, completion_payload, completion_payload_with_meta,
        derive_completion_snapshot, forced_completion_audit, trailing_assistant_text,
        unknown_completion_snapshot,
    };
    use agent_client_protocol_schema::v1::{
        ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent,
    };
    use ora_agent_runtime::PromptEndCause;
    use ora_application::{AgentOutputContract, StructuredTextExposure};
    use ora_history::{HistoryIntegrity, HistoryLine, HistoryRecord, SessionHistory};
    use ora_utils::hash::sha256_hex;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::num::NonZeroUsize;

    /// Builds a history with a complete timeline at zero next seq.
    fn history(lines: Vec<HistoryLine>, integrity: HistoryIntegrity) -> SessionHistory {
        SessionHistory {
            lines,
            next_seq: 0,
            integrity,
        }
    }

    /// Builds one user message line at `seq`.
    fn user(seq: u32, text: &str) -> HistoryLine {
        HistoryLine::new(
            "2026-10-22T10:00:00+08:00",
            seq,
            HistoryRecord::Update {
                update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(text.to_string())),
                ))),
                tool_timing: None,
            },
        )
    }

    /// Builds one assistant message line at `seq`.
    fn assistant(seq: u32, text: &str) -> HistoryLine {
        HistoryLine::new(
            "2026-10-22T10:00:00+08:00",
            seq,
            HistoryRecord::Update {
                update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(text.to_string())),
                ))),
                tool_timing: None,
            },
        )
    }

    /// Builds one `TurnEnded` line at `seq`.
    fn ended(seq: u32, stop_reason: StopReason) -> HistoryLine {
        HistoryLine::new(
            "2026-10-22T10:00:00+08:00",
            seq,
            HistoryRecord::TurnEnded { stop_reason },
        )
    }

    /// A normal completed turn is ready with a full turn key and digest.
    #[test]
    fn completed_turn_is_ready() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "done"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(snapshot.user_seq, Some(1));
        assert_eq!(
            snapshot.turn_key,
            Some(TurnKey {
                user_seq: 1,
                ended_seq: 3
            })
        );
        assert_eq!(snapshot.stop_reason.as_deref(), Some("end_turn"));
        assert!(snapshot.blockers.is_empty());
        assert_eq!(
            snapshot.output_digest,
            Some(format!("sha256:{}", sha256_hex(b"done")))
        );
        assert!(!snapshot.provisional);
    }

    /// A retry only gets credit for evidence after its own boundary, including tool-only output.
    #[test]
    fn a_retried_turn_adopts_only_current_results() {
        use agent_client_protocol_schema::v1::{ToolCall, ToolCallId, ToolCallStatus};
        for (status, recovered) in [
            (ToolCallStatus::Pending, false),
            (ToolCallStatus::Failed, false),
            (ToolCallStatus::Completed, true),
        ] {
            let tool = ToolCall::new(ToolCallId::new("reused"), "deliver").status(status);
            let history = history(
                vec![
                    user(1, "deliver"),
                    assistant(2, "discarded"),
                    HistoryLine::new("", 3, HistoryRecord::PromptRetried),
                    HistoryLine::new(
                        "",
                        4,
                        HistoryRecord::Update {
                            update: Box::new(SessionUpdate::ToolCall(tool)),
                            tool_timing: None,
                        },
                    ),
                    ended(5, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            );
            let snapshot =
                derive_completion_snapshot(&history, None, false, TurnOrigin::Human, None);
            assert_eq!(super::completion_final_text(&history), None);
            assert_eq!(snapshot.blockers.is_empty(), recovered);
            assert_eq!(snapshot.park_kind.is_none(), recovered);
            let mut next = history;
            next.lines.insert(4, assistant(5, "final"));
            *next.lines.last_mut().unwrap() = ended(6, StopReason::EndTurn);
            assert_eq!(
                super::completion_final_text(&next).as_deref(),
                Some("final")
            );
        }
    }

    #[test]
    fn only_parent_text_in_the_final_attempt_can_satisfy_the_retry_guard() {
        let mut child = ContentChunk::new(ContentBlock::Text(TextContent::new("child result")));
        child.meta = Some(agent_client_protocol_schema::v1::Meta::from_iter([(
            "opencode/child-session".to_owned(),
            json!({"id":"child","parentID":"parent","depth":1}),
        )]));
        let history = history(
            vec![
                user(1, "work"),
                HistoryLine::new("", 2, HistoryRecord::PromptRetried),
                assistant(3, "  \n "),
                HistoryLine::new(
                    "",
                    4,
                    HistoryRecord::Update {
                        update: Box::new(SessionUpdate::AgentMessageChunk(child)),
                        tool_timing: None,
                    },
                ),
                ended(5, StopReason::EndTurn),
            ],
            HistoryIntegrity::Complete,
        );
        assert_eq!(
            super::completion_final_text(&history).as_deref(),
            Some("  \n ")
        );
        let snapshot = derive_completion_snapshot(&history, None, false, TurnOrigin::Human, None);
        assert!(snapshot.empty_retry_blocker().is_some());
    }

    #[test]
    fn a_fresh_signal_on_an_empty_opening_retry_still_requires_first_turn_confirmation() {
        let key = TurnKey {
            user_seq: 1,
            ended_seq: 3,
        };
        let h = history(
            vec![
                user(1, "start"),
                HistoryLine::new("", 2, HistoryRecord::PromptRetried),
                ended(3, StopReason::EndTurn),
            ],
            HistoryIntegrity::Complete,
        );
        let snapshot = derive_completion_snapshot(&h, None, false, TurnOrigin::Workflow, None)
            .with_agent_signal_gate(&CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: Some(AgentSignal {
                    signal_id: "fresh".into(),
                    requested_at: 1,
                    turn_key: key,
                }),
                confirmation: None,
                auto_accept: true,
            });
        assert_eq!(
            snapshot.blockers,
            vec![
                CompletionBlocker::new(CompletionBlockerKind::TurnUnconfirmed),
                CompletionBlocker::new(CompletionBlockerKind::AgentConfirmationPending),
            ]
        );
        assert!(
            !snapshot
                .warnings
                .iter()
                .any(|warning| warning == super::RETRY_NO_RESULT)
        );
    }

    /// The workflow's opening turn cannot make the step ready on its own: an `EndTurn` only
    /// finishes that reply, and the reply may be a question waiting for the user.
    #[test]
    fn workflow_initial_turn_is_not_confirmed_complete() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "run the workflow step"),
                    assistant(2, "which of the two existing artifacts should I keep?"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Workflow,
            None,
        );
        assert_eq!(
            snapshot.blockers,
            vec![CompletionBlocker::new(
                CompletionBlockerKind::TurnUnconfirmed
            )]
        );
        // The turn itself is still settled: identity and digest stay available for the conflict
        // checks and for a forced completion that adopts the current output.
        assert_eq!(
            snapshot.turn_key,
            Some(TurnKey {
                user_seq: 1,
                ended_seq: 3
            })
        );
        assert_eq!(
            snapshot.output_digest,
            Some(format!(
                "sha256:{}",
                sha256_hex(b"which of the two existing artifacts should I keep?")
            ))
        );
        assert!(!snapshot.provisional);
    }

    /// The opening turn's park carries the origin and classification both UI entry points read.
    #[test]
    fn the_opening_turn_parks_as_awaiting_user_confirmation() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "run the workflow step"),
                    assistant(2, "which option?"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Workflow,
            None,
        );
        assert_eq!(snapshot.turn_origin, Some(TurnOrigin::Workflow));
        assert_eq!(snapshot.stop_cause, None);
        assert_eq!(snapshot.park_kind, Some(ParkKind::AwaitingUser));
        let payload = completion_payload(&snapshot);
        assert_eq!(payload["completion"]["turnOrigin"], "workflow");
        assert_eq!(payload["completion"]["parkKind"], "awaiting_user");
        assert_eq!(payload["completion"]["stopCause"], json!(null));
    }

    /// The user's own stop keeps waiting for participation, but it is not a confirmation.
    #[test]
    fn a_user_stopped_turn_parks_as_awaiting_user() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "partial"),
                    ended(3, StopReason::Cancelled),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            Some(StopCause::UserStopped),
        );
        assert_eq!(
            snapshot.blockers,
            vec![CompletionBlocker::new(CompletionBlockerKind::TurnStopped)]
        );
        assert_eq!(snapshot.park_kind, Some(ParkKind::AwaitingUser));
    }

    /// A timeout carries the same blocker kind as a user stop but must read as abnormal.
    #[test]
    fn a_timed_out_turn_parks_as_abnormal() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "partial"),
                    ended(3, StopReason::Cancelled),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            Some(StopCause::PromptTimedOut),
        );
        assert_eq!(snapshot.stop_cause, Some(StopCause::PromptTimedOut));
        assert_eq!(snapshot.park_kind, Some(ParkKind::Abnormal));
    }

    /// The runtime's end report maps onto the park's stop cause without inventing one.
    #[test]
    fn the_prompt_end_cause_maps_onto_the_park_stop_cause() {
        for (end, expected) in [
            (None, None),
            (Some(PromptEndCause::Settled), None),
            (
                Some(PromptEndCause::UserStopped),
                Some(StopCause::UserStopped),
            ),
            (
                Some(PromptEndCause::InactivityTimeout),
                Some(StopCause::PromptTimedOut),
            ),
            (
                Some(PromptEndCause::StreamDropped),
                Some(StopCause::StreamDropped),
            ),
            (
                Some(PromptEndCause::SessionCrashed),
                Some(StopCause::SessionCrashed),
            ),
            (Some(PromptEndCause::Unknown), Some(StopCause::Unknown)),
        ] {
            assert_eq!(StopCause::of_prompt_end(end), expected);
        }
    }

    /// A stop whose cause the park site could not determine is never shown as the user's stop.
    #[test]
    fn an_unattributed_stop_parks_as_abnormal() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "partial"),
                    ended(3, StopReason::Cancelled),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(snapshot.park_kind, Some(ParkKind::Abnormal));
    }

    /// Truncation, refusal and output-contract failures are abnormal without a stop cause.
    #[test]
    fn other_blocked_turns_park_as_abnormal() {
        for (last, contract) in [
            (ended(3, StopReason::MaxTokens), None),
            (ended(3, StopReason::Refusal), None),
            (
                ended(3, StopReason::EndTurn),
                Some(AgentOutputContract::Text),
            ),
        ] {
            let snapshot = derive_completion_snapshot(
                &history(
                    vec![user(1, "go"), assistant(2, ""), last],
                    HistoryIntegrity::Complete,
                ),
                contract.as_ref(),
                false,
                TurnOrigin::Human,
                None,
            );
            assert!(!snapshot.blockers.is_empty());
            assert_eq!(snapshot.park_kind, Some(ParkKind::Abnormal));
        }
    }

    /// A provisional park stays with the user until `prepare` settles the turn.
    #[test]
    fn a_provisional_park_stays_with_the_user() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![user(1, "go"), assistant(2, "still working")],
                HistoryIntegrity::Complete,
            ),
            None,
            true,
            TurnOrigin::Workflow,
            None,
        );
        assert!(
            snapshot
                .blockers
                .iter()
                .any(|blocker| { blocker.kind == CompletionBlockerKind::TurnUnknown })
        );
        assert_eq!(snapshot.park_kind, Some(ParkKind::AwaitingUser));
    }

    /// A ready human turn writes no classification and never asks for confirmation.
    #[test]
    fn a_ready_turn_has_no_park_kind() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "done"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(snapshot.park_kind, None);
    }

    /// A human turn keeps the completion ADR rule: a settled `EndTurn` with no blockers is ready.
    #[test]
    fn human_turn_after_the_first_reply_is_ready() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "run the workflow step"),
                    assistant(2, "which option?"),
                    ended(3, StopReason::EndTurn),
                    user(4, "keep the first one"),
                    assistant(5, "done"),
                    ended(6, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert!(snapshot.blockers.is_empty());
    }

    /// A more precise blocker already names why the opening turn cannot complete.
    #[test]
    fn a_stopped_opening_turn_keeps_only_its_own_blocker() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "partial"),
                    ended(3, StopReason::Cancelled),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Workflow,
            None,
        );
        assert_eq!(
            snapshot.blockers,
            vec![CompletionBlocker::new(CompletionBlockerKind::TurnStopped)]
        );
    }

    /// Stopped, truncated and refused turns each report their own blocker.
    #[test]
    fn non_end_turns_are_blocked_by_kind() {
        for (reason, expected) in [
            (StopReason::Cancelled, CompletionBlockerKind::TurnStopped),
            (StopReason::MaxTokens, CompletionBlockerKind::TurnTruncated),
            (
                StopReason::MaxTurnRequests,
                CompletionBlockerKind::TurnTruncated,
            ),
            (StopReason::Refusal, CompletionBlockerKind::TurnRefused),
        ] {
            let snapshot = derive_completion_snapshot(
                &history(
                    vec![user(1, "go"), assistant(2, "partial"), ended(3, reason)],
                    HistoryIntegrity::Complete,
                ),
                None,
                false,
                TurnOrigin::Human,
                None,
            );
            assert_eq!(
                snapshot.blockers.first().map(|blocker| blocker.kind),
                Some(expected),
                "unexpected blocker for {reason:?}"
            );
        }
    }

    /// A history without a user message has no turn and cannot be ready.
    #[test]
    fn missing_user_message_is_unknown() {
        let snapshot = derive_completion_snapshot(
            &history(vec![assistant(1, "hello")], HistoryIntegrity::Complete),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(snapshot.user_seq, None);
        assert_eq!(snapshot.turn_key, None);
        assert_eq!(snapshot.output_digest, None);
        assert_eq!(
            snapshot.blockers.first().map(|blocker| blocker.kind),
            Some(CompletionBlockerKind::TurnUnknown)
        );
    }

    /// A user message without a `TurnEnded` is unknown and has no digest.
    #[test]
    fn missing_turn_ended_is_unknown() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![user(1, "go"), assistant(2, "still going")],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(snapshot.turn_key, None);
        assert_eq!(snapshot.output_digest, None);
        assert_eq!(
            snapshot.blockers.first().map(|blocker| blocker.kind),
            Some(CompletionBlockerKind::TurnUnknown)
        );
    }

    /// The final text is the last assistant message in the window, even when it is empty, not the
    /// last non-empty message.
    #[test]
    fn final_text_is_the_last_assistant_message_even_when_empty() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "progress"),
                    assistant(3, ""),
                    ended(4, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            Some(&AgentOutputContract::Text),
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(
            snapshot.blockers.first().map(|blocker| blocker.kind),
            Some(CompletionBlockerKind::OutputMissing)
        );
        assert_eq!(
            snapshot.output_digest,
            Some(format!("sha256:{}", sha256_hex(b"")))
        );
    }

    /// Empty output is only a warning for the legacy no-contract node.
    #[test]
    fn none_contract_empty_output_is_a_warning() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![user(1, "go"), ended(3, StopReason::EndTurn)],
                HistoryIntegrity::Complete,
            ),
            Some(&AgentOutputContract::None),
            false,
            TurnOrigin::Human,
            None,
        );
        assert!(snapshot.blockers.is_empty());
        assert_eq!(snapshot.warnings, vec!["empty_output".to_string()]);
    }

    /// A structured contract failure is a readable blocker, not a terminal error.
    #[test]
    fn structured_failure_is_a_blocker() {
        let schema = json!({
            "type": "object",
            "required": ["answer"],
            "properties": { "answer": { "type": "string" } }
        });
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "not json"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            Some(&AgentOutputContract::Structured {
                schema,
                text_exposure: StructuredTextExposure::IncludeFinalText,
            }),
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(
            snapshot.blockers.first().map(|blocker| blocker.kind),
            Some(CompletionBlockerKind::StructuredInvalid)
        );
    }

    /// Damaged history keeps the found turn key but still blocks as unknown with a warning.
    #[test]
    fn damaged_history_stays_unknown() {
        let unreadable = NonZeroUsize::new(1).unwrap();
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "done"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Damaged {
                    unreadable_lines: unreadable,
                },
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(
            snapshot.turn_key,
            Some(TurnKey {
                user_seq: 1,
                ended_seq: 3
            })
        );
        assert_eq!(
            snapshot.blockers.first().map(|blocker| blocker.kind),
            Some(CompletionBlockerKind::TurnUnknown)
        );
        assert!(
            snapshot
                .warnings
                .iter()
                .any(|warning| warning.starts_with("history_integrity_gap"))
        );
    }

    /// The persisted payload carries the snapshot under the `completion` key.
    #[test]
    fn payload_wraps_the_snapshot() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "done"),
                    ended(3, StopReason::EndTurn),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            true,
            TurnOrigin::Human,
            None,
        );
        let payload = completion_payload(&snapshot);
        assert_eq!(payload["completion"]["userSeq"], 1);
        assert_eq!(payload["completion"]["provisional"], true);
    }

    /// A stray assistant message before the last user message is not the forced output.
    #[test]
    fn trailing_assistant_text_stays_inside_the_last_turn() {
        assert_eq!(
            trailing_assistant_text(&history(
                vec![
                    user(1, "first"),
                    assistant(2, "first answer"),
                    user(3, "second"),
                    assistant(4, "second answer"),
                ],
                HistoryIntegrity::Complete,
            )),
            Some("second answer".to_string())
        );
        // A user message with no reply yet is still the current turn, so nothing is borrowed.
        assert_eq!(
            trailing_assistant_text(&history(
                vec![
                    user(1, "first"),
                    assistant(2, "first answer"),
                    user(3, "second")
                ],
                HistoryIntegrity::Complete,
            )),
            None
        );
    }

    /// The audit records the blockers and names where a forced output came from.
    #[test]
    fn forced_audit_names_the_output_source() {
        let settled = derive_completion_snapshot(
            &history(
                vec![
                    user(1, "go"),
                    assistant(2, "done"),
                    ended(3, StopReason::Cancelled),
                ],
                HistoryIntegrity::Complete,
            ),
            None,
            false,
            TurnOrigin::Human,
            None,
        );
        assert_eq!(
            forced_completion_audit(&settled, &settled.blockers)["completion_forced"],
            json!({
                "by": "human",
                "blockers": [{ "kind": "turn_stopped" }],
                "stop_cause": null,
                "without_turn": false,
                "output_source": "turn_window",
            })
        );

        let unsettled =
            unknown_completion_snapshot(true, TurnOrigin::Human, Some(StopCause::Unknown));
        assert_eq!(
            forced_completion_audit(&unsettled, &unsettled.blockers)["completion_forced"],
            json!({
                "by": "human",
                "blockers": [{ "kind": "turn_unknown" }],
                "stop_cause": "unknown",
                "without_turn": true,
                "output_source": "last_assistant_message",
            })
        );
    }

    /// A turn with no settled boundary reports no output-contract warning or blocker of its own.
    #[test]
    fn an_unsettled_turn_does_not_report_output_contract_problems() {
        let snapshot = derive_completion_snapshot(
            &history(
                vec![user(1, "go"), assistant(2, "still working")],
                HistoryIntegrity::Complete,
            ),
            Some(&AgentOutputContract::Text),
            false,
            TurnOrigin::Human,
            None,
        );
        assert!(snapshot.warnings.is_empty());
        assert_eq!(
            snapshot.blockers.first().map(|blocker| blocker.kind),
            Some(CompletionBlockerKind::TurnUnknown)
        );
    }

    /// Legacy payloads have no policy and read as end-turn; agent metadata round-trips beside it.
    #[test]
    fn completion_meta_defaults_to_end_turn_and_round_trips_agent_fields() {
        let snapshot = unknown_completion_snapshot(false, TurnOrigin::Human, None);
        let default_meta = CompletionMeta::default();
        assert_eq!(default_meta.policy, CompletionPolicy::EndTurn);
        let payload = completion_payload_with_meta(&snapshot, &default_meta);
        assert_eq!(payload["completionPolicy"], json!("end_turn"));
        assert_eq!(
            CompletionMeta::from_payload(Some(&payload.to_string())),
            default_meta
        );

        let meta = CompletionMeta {
            policy: CompletionPolicy::Agent,
            agent_signal: Some(AgentSignal {
                signal_id: "signal-1".to_string(),
                requested_at: 42,
                turn_key: TurnKey {
                    user_seq: 1,
                    ended_seq: 2,
                },
            }),
            confirmation: Some(CompletionConfirmation {
                by: ConfirmationBy::Human,
                at: 43,
            }),
            auto_accept: true,
        };
        let payload = completion_payload_with_meta(&snapshot, &meta);
        assert_eq!(payload["agentSignal"]["signalId"], json!("signal-1"));
        assert_eq!(payload["confirmation"]["by"], json!("human"));
        assert_eq!(payload["autoAcceptAgentCompletion"], json!(true));
        assert_eq!(
            CompletionMeta::from_payload(Some(&payload.to_string())),
            meta
        );
    }

    /// A settled agent-policy turn materializes its signal gate into the persisted blockers.
    #[test]
    fn agent_policy_materializes_the_signal_gate() {
        let snapshot = CompletionSnapshot {
            user_seq: Some(1),
            turn_key: Some(TurnKey {
                user_seq: 1,
                ended_seq: 2,
            }),
            stop_reason: Some("end_turn".to_string()),
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: None,
            park_kind: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
            output_digest: None,
            provisional: false,
        };
        let missing = completion_payload_with_meta(
            &snapshot,
            &CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: None,
                confirmation: None,
                auto_accept: false,
            },
        );
        assert_eq!(
            missing["completion"]["blockers"],
            json!([{ "kind": "agent_signal_pending" }])
        );
        assert_eq!(missing["completion"]["parkKind"], json!("awaiting_user"));
        assert_eq!(missing["agentSignal"], serde_json::Value::Null);
        assert_eq!(missing["confirmation"], serde_json::Value::Null);

        let signaled = completion_payload_with_meta(
            &snapshot,
            &CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: Some(AgentSignal {
                    signal_id: "signal-1".to_string(),
                    requested_at: 42,
                    turn_key: TurnKey {
                        user_seq: 1,
                        ended_seq: 2,
                    },
                }),
                confirmation: None,
                auto_accept: false,
            },
        );
        assert_eq!(
            signaled["completion"]["blockers"],
            json!([{ "kind": "agent_confirmation_pending" }])
        );
        assert_eq!(signaled["completion"]["parkKind"], json!("awaiting_user"));
        assert_eq!(signaled["agentSignal"]["signalId"], json!("signal-1"));
    }

    /// A precise stop owns the gate; an accepted signal must not add a confirmation blocker.
    #[test]
    fn a_precise_stop_keeps_its_blockers_without_the_signal_gate() {
        let snapshot = CompletionSnapshot {
            user_seq: Some(1),
            turn_key: Some(TurnKey {
                user_seq: 1,
                ended_seq: 2,
            }),
            stop_reason: Some("cancelled".to_string()),
            turn_origin: Some(TurnOrigin::Human),
            stop_cause: Some(StopCause::PromptTimedOut),
            park_kind: Some(ParkKind::Abnormal),
            blockers: vec![CompletionBlocker::new(CompletionBlockerKind::TurnStopped)],
            warnings: Vec::new(),
            output_digest: None,
            provisional: false,
        };
        let payload = completion_payload_with_meta(
            &snapshot,
            &CompletionMeta {
                policy: CompletionPolicy::Agent,
                agent_signal: Some(AgentSignal {
                    signal_id: "signal-1".to_string(),
                    requested_at: 42,
                    turn_key: TurnKey {
                        user_seq: 1,
                        ended_seq: 2,
                    },
                }),
                confirmation: None,
                auto_accept: true,
            },
        );
        assert_eq!(
            payload["completion"]["blockers"],
            json!([{ "kind": "turn_stopped" }]),
            "the precise stop wins over an accepted signal"
        );
        assert_eq!(payload["completion"]["parkKind"], json!("abnormal"));
    }
}

//! Derives the current interactive turn's completion evidence from durable session history.

use super::retry_output::last_retry_seq;
use super::{
    CompletionBlocker, CompletionBlockerKind, CompletionSnapshot, RETRY_NO_RESULT, StopCause,
    TurnKey, TurnOrigin, output_contract_blocker, park_kind, retry_output,
};
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, StopReason};
use ora_application::AgentOutputContract;
use ora_history::{HistoryIntegrity, HistoryRecord, SessionHistory};
use ora_utils::hash::sha256_hex;

/// Returns the last assistant text after the last user message, whether or not a turn settled.
///
/// This is the forced-completion text for a snapshot with no turn key: the contract forbids
/// borrowing a previous turn's `EndTurn`, but the user still asked for the current draft to be
/// taken as the output.
pub(crate) fn trailing_assistant_text(history: &SessionHistory) -> Option<String> {
    let user_seq = last_user_message_seq(history)?;
    final_assistant_text(history, user_seq, u32::MAX)
}

/// Returns the final assistant text of the last settled turn, using the same window as the
/// snapshot derivation so the completion commit and its digest cannot disagree.
pub(crate) fn completion_final_text(history: &SessionHistory) -> Option<String> {
    let user_seq = last_user_message_seq(history)?;
    let (ended_seq, _) = settled_turn(history, user_seq)?;
    final_assistant_text(history, user_seq, ended_seq)
}

/// Derives the completion snapshot for the last settled turn in `history`.
///
/// The final text window is `(userSeq, endedSeq]` and takes the last assistant message by `seq`,
/// including an empty one, matching the contract rather than the legacy "last non-empty" helper.
/// A retry narrows the output window to the final send while retaining the original turn key.
///
/// `origin` marks the workflow's opening turn: when that turn is otherwise ready, the snapshot
/// carries `turn_unconfirmed` instead of reporting the step ready, because the agent may have
/// ended its first reply with a question the user still has to answer. `stop_cause` is the park
/// site's knowledge of why the owning prompt ended; it only shapes the display classification and
/// the forced-completion audit, never the blockers.
pub(crate) fn derive_completion_snapshot(
    history: &SessionHistory,
    output_contract: Option<&AgentOutputContract>,
    provisional: bool,
    origin: TurnOrigin,
    stop_cause: Option<StopCause>,
) -> CompletionSnapshot {
    let user_seq = last_user_message_seq(history);
    let turn = user_seq.and_then(|user_seq| settled_turn(history, user_seq));
    let mut warnings = Vec::new();
    let mut blockers = Vec::new();

    let (turn_key, stop_reason) = match (user_seq, turn) {
        (Some(user_seq), Some((ended_seq, reason))) => (
            Some(TurnKey {
                user_seq,
                ended_seq,
            }),
            Some(super::super::executor::stop_reason_label(reason)),
        ),
        _ => (None, None),
    };

    if let HistoryIntegrity::Damaged { unreadable_lines } = history.integrity {
        warnings.push(format!(
            "history_integrity_gap: {unreadable_lines} unreadable history line(s)"
        ));
        blockers.push(CompletionBlocker::new(CompletionBlockerKind::TurnUnknown));
    } else if turn.is_none() {
        blockers.push(CompletionBlocker::new(CompletionBlockerKind::TurnUnknown));
    }

    let final_text = user_seq
        .zip(turn)
        .and_then(|(user, (end, _))| final_assistant_text(history, user, end));
    if let (Some(user), Some((ended, StopReason::EndTurn))) = (user_seq, turn)
        && let Some(start) = last_retry_seq(history, user, ended)
        && final_text
            .as_deref()
            .is_none_or(|text| text.trim().is_empty())
        && !retry_output::has_non_text_result(history, user, start, ended)
    {
        warnings.push(RETRY_NO_RESULT.to_string());
    }
    let output_digest = turn.map(|_| {
        format!(
            "sha256:{}",
            sha256_hex(final_text.as_deref().unwrap_or_default().as_bytes())
        )
    });

    if let Some(stop_reason) = turn.map(|(_, reason)| reason) {
        match stop_reason {
            StopReason::EndTurn => {}
            StopReason::MaxTokens | StopReason::MaxTurnRequests => {
                blockers.push(CompletionBlocker::new(CompletionBlockerKind::TurnTruncated));
            }
            StopReason::Cancelled => {
                blockers.push(CompletionBlocker::new(CompletionBlockerKind::TurnStopped));
            }
            StopReason::Refusal => {
                blockers.push(CompletionBlocker::new(CompletionBlockerKind::TurnRefused));
            }
            _ => blockers.push(CompletionBlocker::new(CompletionBlockerKind::TurnUnknown)),
        }
    }

    // A missing final message is an empty final text; the contract decides whether that blocks.
    // Without a settled turn there is no output to judge at all: `turn_unknown` already blocks,
    // and an `empty_output` warning would describe a turn the history never closed.
    let final_text = final_text.unwrap_or_default();
    if turn.is_some() {
        if matches!(output_contract, None | Some(AgentOutputContract::None))
            && final_text.is_empty()
            && !warnings.iter().any(|warning| warning == RETRY_NO_RESULT)
        {
            warnings.push("empty_output".to_string());
        }
        if let Some(blocker) = output_contract_blocker(output_contract, Some(final_text.as_str())) {
            blockers.push(blocker);
        } else if warnings.iter().any(|warning| warning == RETRY_NO_RESULT) {
            blockers.push(CompletionBlocker {
                kind: CompletionBlockerKind::OutputMissing,
                params: serde_json::json!({ "reason": RETRY_NO_RESULT }),
            });
        }
    }

    // The workflow's opening turn never makes the step ready by itself: its `EndTurn` only means
    // the agent finished that reply, which may have been a question. The user must answer or
    // explicitly adopt the output, so the node stays open. A more precise blocker already keeps
    // the node open and names the actual reason, so only an otherwise-ready snapshot gets the
    // confirmation marker.
    if matches!(origin, TurnOrigin::Workflow) && !provisional && blockers.is_empty() {
        blockers.push(CompletionBlocker::new(
            CompletionBlockerKind::TurnUnconfirmed,
        ));
    }

    CompletionSnapshot {
        user_seq,
        turn_key,
        stop_reason,
        turn_origin: Some(origin),
        stop_cause,
        park_kind: park_kind(&blockers, provisional, stop_cause),
        blockers,
        warnings,
        output_digest,
        provisional,
    }
}

/// Returns the `seq` of the last user message, which opens the turn under evaluation.
fn last_user_message_seq(history: &SessionHistory) -> Option<u32> {
    history
        .lines
        .iter()
        .filter(|line| {
            matches!(
                &line.record,
                HistoryRecord::Update { update, .. }
                    if matches!(update.as_ref(), SessionUpdate::UserMessageChunk(_))
            )
        })
        .map(|line| line.seq)
        .max()
}

/// Returns the last `TurnEnded` after `user_seq` with its position.
fn settled_turn(history: &SessionHistory, user_seq: u32) -> Option<(u32, StopReason)> {
    history
        .lines
        .iter()
        .filter_map(|line| {
            let HistoryRecord::TurnEnded { stop_reason } = &line.record else {
                return None;
            };
            (line.seq > user_seq).then_some((line.seq, *stop_reason))
        })
        .max_by_key(|(seq, _)| *seq)
}

/// Returns the last assistant message inside `(user_seq, ended_seq]`, including an empty one.
fn final_assistant_text(history: &SessionHistory, user_seq: u32, ended_seq: u32) -> Option<String> {
    let retry_seq = last_retry_seq(history, user_seq, ended_seq);
    let start = retry_seq.unwrap_or(user_seq);
    history
        .lines
        .iter()
        .filter_map(|line| {
            if line.seq <= start || line.seq > ended_seq {
                return None;
            }
            let HistoryRecord::Update { update, .. } = &line.record else {
                return None;
            };
            if retry_seq.is_some()
                && matches!(update.as_ref(), SessionUpdate::AgentMessageChunk(chunk)
                if ora_history::child_source_id(chunk.meta.as_ref()).is_some())
            {
                return None;
            }
            assistant_text(update.as_ref()).map(str::to_string)
        })
        .next_back()
}

/// Extracts the text block of one settled assistant message.
fn assistant_text(update: &SessionUpdate) -> Option<&str> {
    let SessionUpdate::AgentMessageChunk(chunk) = update else {
        return None;
    };
    let ContentBlock::Text(text) = &chunk.content else {
        return None;
    };
    Some(&text.text)
}

impl CompletionSnapshot {
    /// Carries the retry guard from live history through settlement of a previously stored snapshot.
    pub(crate) fn empty_retry_blocker(&self) -> Option<CompletionBlocker> {
        self.blockers
            .iter()
            .find(|blocker| blocker.params["reason"] == RETRY_NO_RESULT)
            .cloned()
    }

    /// Rechecks current output requirements alongside the persisted park decision.
    pub(crate) fn live_output_blockers(
        &self,
        retry_blocker: Option<CompletionBlocker>,
        output_contract: Option<&AgentOutputContract>,
        raw_output: Option<&str>,
    ) -> Vec<CompletionBlocker> {
        let mut effective_blockers = self.blockers.clone();
        if let Some(blocker) = retry_blocker
            && !effective_blockers.contains(&blocker)
        {
            effective_blockers.push(blocker);
        }
        if self.turn_key.is_some()
            && let Some(blocker) = output_contract_blocker(output_contract, raw_output)
            && !effective_blockers
                .iter()
                .any(|existing| existing.kind == blocker.kind)
        {
            effective_blockers.push(blocker);
        }
        effective_blockers
    }
}

//! What the session remembers about its cancels and turns outside any one turn: which turns
//! already have their own result, when each pending cancel was accepted, and the settlement of
//! cancels that arrive before the Agent starts (cancel-turn D1).
use super::*;
use ora_node_protocol::{TurnCancelOutcome, TurnId};

/// Which turns of the session already have their own result, kept from before the agent starts
/// (a prior Revision restore can take long) through the whole conversation.
#[derive(Default)]
pub(in crate::session::driver) struct TurnBook {
    ended: std::sync::Mutex<std::collections::HashSet<TurnId>>,
    /// The initial turn was dropped by a cancel before the conversation could start it.
    initial_dropped: std::sync::atomic::AtomicBool,
}

impl TurnBook {
    /// Whether the turn already has its own result.
    pub(super) fn has_ended(&self, turn: &TurnId) -> bool {
        self.ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(turn)
    }

    /// Remembers that the turn has its own result.
    pub(super) fn ended(&self, turn: &TurnId) {
        self.ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(turn.clone());
    }

    /// Whether the conversation must skip the initial turn.
    pub(in crate::session::driver) fn initial_dropped(&self) -> bool {
        self.initial_dropped
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Settles every queued cancel while the session prepares, before its agent can start: no turn
/// has been admitted yet, so the initial turn and every queued turn are dropped as `not_started`
/// at once, without waiting for a restore or the agent (D1). An `EndSession` takes precedence and
/// its decision settles the cancels instead.
pub(in crate::session::driver) fn settle_before_start<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
    initial: &TurnId,
    turns: &TurnBook,
    acceptance: &Acceptance,
) -> Result<(), SessionEnd> {
    let unavailable = || SessionEnd::agent_failed("ledger_unavailable");
    let queued = ledger
        .queued_commands(execution)
        .map_err(|_| unavailable())?;
    let ending = |ledger: &L| {
        ledger.queued_commands(execution).is_ok_and(|queued| {
            queued
                .iter()
                .any(|queued| matches!(queued.command, SessionCommand::EndSession(_)))
        })
    };
    if ending(ledger) {
        return Ok(());
    }
    for queued_cancel in &queued {
        let SessionCommand::CancelTurn(turn_id) = &queued_cancel.command else {
            continue;
        };
        let target = queued.iter().find_map(|queued| match &queued.command {
            SessionCommand::SubmitUserTurn(turn) if &turn.turn_id == turn_id => {
                Some(queued.command_id.clone())
            }
            SessionCommand::SubmitUserTurn(_)
            | SessionCommand::EndSession(_)
            | SessionCommand::CancelTurn(_) => None,
        });
        let initial_waiting = turn_id == initial && !turns.has_ended(turn_id);
        let outcome = if target.is_some() || initial_waiting {
            TurnCancelOutcome::NotStarted
        } else if turns.has_ended(turn_id) {
            TurnCancelOutcome::AlreadyEnded
        } else {
            TurnCancelOutcome::Unknown
        };
        match ledger.settle_turn_cancel(
            execution,
            &queued_cancel.command_id,
            outcome,
            target.as_ref(),
        ) {
            Ok(TurnCancelOutcome::NotStarted) => {
                forget_acceptance(acceptance, execution, &queued_cancel.command_id);
                turns.ended(turn_id);
                if initial_waiting {
                    turns
                        .initial_dropped
                        .store(true, std::sync::atomic::Ordering::Release);
                }
            }
            Ok(_) => forget_acceptance(acceptance, execution, &queued_cancel.command_id),
            Err(_) if ending(ledger) => return Ok(()),
            Err(_) => return Err(unavailable()),
        }
    }
    Ok(())
}

/// Acceptance times of pending cancels, by execution and command.
pub(in crate::session) type Acceptance =
    std::sync::Mutex<std::collections::HashMap<(ExecutionId, CommandId), tokio::time::Instant>>;

/// Drops the acceptance time of a cancel that has its settlement.
pub(super) fn forget_acceptance(
    acceptance: &Acceptance,
    execution: &ExecutionId,
    command: &CommandId,
) {
    acceptance
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&(execution.clone(), command.clone()));
}

/// The latest moment by which the pending cancels need an end decision: the earliest
/// acceptance plus G. A cancel accepted before a restart has no recorded time and counts from
/// now.
pub(in crate::session::driver) fn pending_cancel_bound<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
    acceptance: &Acceptance,
    grace: std::time::Duration,
) -> Result<Option<tokio::time::Instant>, &'static str> {
    let queued = ledger
        .queued_commands(execution)
        .map_err(|_| "ledger_unavailable")?;
    let times = acceptance
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(queued
        .iter()
        .filter(|queued| matches!(queued.command, SessionCommand::CancelTurn(_)))
        .map(|queued| {
            times
                .get(&(execution.clone(), queued.command_id.clone()))
                .copied()
                .unwrap_or_else(tokio::time::Instant::now)
                + grace
        })
        .min())
}

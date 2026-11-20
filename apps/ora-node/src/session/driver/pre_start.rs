//! Commands that take effect before a turn is admitted: an `EndSession` during a prior Revision
//! restore, and cancels throughout the preparation, which no turn can wait on (cancel-turn D1).
use super::conversation::{Acceptance, TurnBook, settle_before_start};
use super::{CommandSettlement, Plan, SessionEnd, SessionLedger, plan, settle};
use ora_logging::ora_warn;
use ora_node_protocol::ExecutionId;
use tokio::sync::Notify;

/// Resolves once an `EndSession` is queued for a session whose agent has not started: turns
/// queued ahead of it are discarded, and the command is executed by ending the session.
///
/// Cancels take effect here too, so a long restore never delays a cancel's settlement.
pub(super) async fn ended_before_start<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
    wake: &Notify,
    initial: &ora_node_protocol::TurnId,
    turns: &TurnBook,
    acceptance: &Acceptance,
) -> SessionEnd {
    loop {
        if let Err(end) = settle_before_start(ledger, execution, initial, turns, acceptance) {
            return end;
        }
        match ledger.queued_commands(execution).map(plan) {
            Ok(Plan::End {
                command_id,
                reason,
                discarded,
            }) => {
                for discarded in &discarded {
                    settle(ledger, execution, discarded, CommandSettlement::Discarded);
                }
                settle(ledger, execution, &command_id, CommandSettlement::Executed);
                return SessionEnd::requested(reason);
            }
            // A turn waits for the agent; the conversation reads the queue again once it starts.
            Ok(Plan::Turn { .. } | Plan::Wait) => wake.notified().await,
            Err(error) => {
                ora_warn!(execution_id = %execution, error = %error, "session queue unreadable");
                return SessionEnd::agent_failed("ledger_unavailable");
            }
        }
    }
}

/// Clears the acceptance times an execution left behind when its run ends.
pub(super) struct ForgetAcceptances<'a> {
    pub(super) acceptance: &'a Acceptance,
    pub(super) execution: &'a ExecutionId,
}

impl Drop for ForgetAcceptances<'_> {
    fn drop(&mut self) {
        self.acceptance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(accepted, _), _| accepted != self.execution);
    }
}

/// What settling cancels before admission needs.
pub(super) struct PreStart<'a, L> {
    pub(super) ledger: &'a L,
    pub(super) execution: &'a ExecutionId,
    pub(super) wake: &'a Notify,
    pub(super) initial: &'a ora_node_protocol::TurnId,
    pub(super) turns: &'a TurnBook,
    pub(super) acceptance: &'a Acceptance,
}

/// Runs one preparation step while cancels keep taking effect: no turn is admitted yet, so each
/// one settles at once instead of waiting for the step (D1).
pub(super) async fn settling_cancels<L: SessionLedger, F: std::future::Future>(
    step: F,
    pre_start: PreStart<'_, L>,
) -> Result<F::Output, SessionEnd> {
    tokio::pin!(step);
    loop {
        let notified = pre_start.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        settle_before_start(
            pre_start.ledger,
            pre_start.execution,
            pre_start.initial,
            pre_start.turns,
            pre_start.acceptance,
        )?;
        tokio::select! {
            output = &mut step => return Ok(output),
            () = &mut notified => {}
        }
    }
}

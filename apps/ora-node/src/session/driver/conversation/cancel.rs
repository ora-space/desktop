//! `CancelTurn` inside the conversation: where each cancel takes effect, and whether a turn that
//! stopped by cancel or failure leaves the session usable (cancel-turn D1–D4).
//!
//! The ledger is the serial settlement boundary. A cancel of a queued turn, the turn's admission
//! and its MCP wait timeout each commit there, so whichever commits first decides the turn; the
//! conversation only decides when a cancel takes effect and what outcome it asks the ledger for.
use super::*;
use ora_node_protocol::{TurnCancelOutcome, TurnId};

/// A queued `CancelTurn` the conversation has not settled yet.
#[derive(Clone, Debug)]
pub(super) struct PendingCancel {
    pub(super) command_id: CommandId,
}

/// Which side ended a turn that stopped without a normal completion.
#[derive(Clone, Copy, Debug)]
pub(super) enum StopCause {
    /// A `CancelTurn` stopped it and the Agent answered by the deadline.
    Cancel,
    /// The Agent answered the prompt with an error, or the watchdog gave up after an answered
    /// cancel; `code` is the detail the session ends with when it is not kept.
    Failure(&'static str),
}

/// A turn the Agent answered after a cancel or with a failure, waiting for the D2 judgement.
pub(super) struct StoppedTurn {
    pub(super) turn_id: TurnId,
    pub(super) cause: StopCause,
    /// Cancels merged into this stop, in acceptance order.
    pub(super) merged: Vec<CommandId>,
}

/// What the D2 judgement concluded.
pub(super) enum Judged {
    /// The session continues; every merged cancel is settled.
    Kept,
    /// The session ends; merged cancels are settled as `session_ending` with the end decision.
    End(SessionEnd),
    /// An `EndSession` arrived while the turn was being judged.
    EndRequested(TurnOutcome),
}

impl<L: SessionLedger> Conversation<'_, L> {
    /// Settles every queued cancel that does not target `current` and returns those that do.
    ///
    /// An accepted `EndSession` takes precedence over everything queued before it, and its end
    /// decision settles the remaining cancels, so nothing is settled while one is queued.
    pub(super) fn settle_cancels(
        &self,
        current: Option<&TurnId>,
    ) -> Result<Vec<PendingCancel>, SessionEnd> {
        let queued = self
            .ledger
            .queued_commands(self.execution)
            .map_err(|_| SessionEnd::agent_failed("ledger_unavailable"))?;
        if queued
            .iter()
            .any(|queued| matches!(queued.command, SessionCommand::EndSession(_)))
        {
            return Ok(Vec::new());
        }
        let mut targeting = Vec::new();
        for queued_cancel in &queued {
            let SessionCommand::CancelTurn(turn_id) = &queued_cancel.command else {
                continue;
            };
            if Some(turn_id) == current {
                targeting.push(PendingCancel {
                    command_id: queued_cancel.command_id.clone(),
                });
                continue;
            }
            let target = queued.iter().find_map(|queued| match &queued.command {
                SessionCommand::SubmitUserTurn(turn) if &turn.turn_id == turn_id => {
                    Some(queued.command_id.clone())
                }
                SessionCommand::SubmitUserTurn(_)
                | SessionCommand::EndSession(_)
                | SessionCommand::CancelTurn(_) => None,
            });
            let outcome = match &target {
                // Queued behind other turns: dropped now, so its settlement never waits for the
                // turns ahead of it.
                Some(_) => TurnCancelOutcome::NotStarted,
                None if self.has_ended(turn_id) => TurnCancelOutcome::AlreadyEnded,
                None => TurnCancelOutcome::Unknown,
            };
            let recorded = self.settle_one(&queued_cancel.command_id, outcome, target.as_ref())?;
            if recorded == TurnCancelOutcome::NotStarted {
                self.turn_ended(turn_id);
            }
        }
        Ok(targeting)
    }

    /// Settles cancels of the turn waiting for admission. Returns whether the turn was dropped.
    ///
    /// Runs under the admission gate, so it is serialized against the Actor's commit and the MCP
    /// wait timeout. A wait that already reached its deadline is settled as the timeout first
    /// even when the timeout task has not run yet, so a cancel never rescues a turn past the MCP
    /// wait bound; the cancels then find the turn already ended.
    pub(super) fn cancel_waiting(
        &self,
        turn: &UserTurn,
        command: Option<&CommandId>,
        gate: Option<&std::sync::Mutex<admission::TurnGate>>,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<WaitingCancel, SessionEnd> {
        let cancels = self.settle_cancels(Some(&turn.turn_id))?;
        if cancels.is_empty() {
            return Ok(WaitingCancel::None);
        }
        let mut guard = gate.map(|gate| {
            gate.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        if guard.as_ref().is_some_and(|gate| gate.committed) {
            // Admission committed first: the turn has started and the running path cancels it.
            return Ok(WaitingCancel::Started);
        }
        if let Some(gate) = guard.as_mut() {
            gate.expired = true;
        }
        let timed_out = deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline);
        if timed_out {
            self.mirror.set_turn(None);
            self.ledger
                .fail_session_turn(
                    self.execution,
                    ora_node_protocol::SessionTurnFailed {
                        turn_id: turn.turn_id.clone(),
                        command_id: command.cloned(),
                        code: ora_node_protocol::SessionTurnFailureCode::McpRefreshWaitTimeout,
                    },
                )
                .or_else(|_| match self.queued_end()? {
                    Some(_) => Ok(()),
                    None => Err(SessionEnd::agent_failed("ledger_unavailable")),
                })?;
            if self.queued_end()?.is_some() {
                return Ok(WaitingCancel::None);
            }
        }
        let mut first = !timed_out;
        for cancel in cancels {
            // The first cancel drops the turn; later ones find it dropped by an earlier cancel.
            let outcome = if first {
                TurnCancelOutcome::NotStarted
            } else {
                TurnCancelOutcome::AlreadyEnded
            };
            first = false;
            match self.ledger.settle_turn_cancel(
                self.execution,
                &cancel.command_id,
                outcome,
                command.filter(|_| outcome == TurnCancelOutcome::NotStarted),
            ) {
                Ok(_) => {
                    forget_acceptance(self.cancel_acceptance, self.execution, &cancel.command_id)
                }
                Err(_) if self.queued_end()?.is_some() => return Ok(WaitingCancel::None),
                Err(_) => return Err(SessionEnd::agent_failed("ledger_unavailable")),
            }
        }
        drop(guard);
        self.blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&turn.turn_id);
        self.mirror.set_turn(None);
        self.turn_ended(&turn.turn_id);
        Ok(WaitingCancel::Dropped)
    }

    /// Starts stopping the running turn for its first cancel; later cancels share the deadline.
    pub(super) fn request_stop(&self, merged: &mut Vec<CommandId>, cancels: Vec<PendingCancel>) {
        for cancel in cancels {
            if merged.contains(&cancel.command_id) {
                continue;
            }
            // Read, not taken: the time stays until the cancel is settled, so an end decided
            // before then still knows when the cancel's deadline falls.
            let accepted_at = self
                .cancel_acceptance
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&(self.execution.clone(), cancel.command_id.clone()))
                .copied();
            if merged.is_empty() {
                // The deadline counts from the first cancel's durable acceptance and never
                // moves; the runtime keeps the first deadline it is given. A cancel accepted
                // before a restart has no recorded time and counts from now.
                let deadline =
                    accepted_at.unwrap_or_else(tokio::time::Instant::now) + self.turn_cancel.grace;
                if let Err(error) = self
                    .manager
                    .cancel_turn_prompt(self.session_id.as_ref(), deadline)
                {
                    ora_warn!(execution_id = %self.execution, error = %error, "turn cancel could not reach the runtime");
                }
            }
            merged.push(cancel.command_id);
        }
    }

    /// Applies D2 to a stopped turn: the session is kept only if it chose to keep after this
    /// kind of end, the Agent version has a verified turn boundary, the turn left nothing
    /// running, and no turn traffic arrives within the quiet window.
    pub(super) async fn judge(&self, mut stopped: StoppedTurn) -> Judged {
        let chosen = match stopped.cause {
            StopCause::Cancel => self.keep.on_cancel,
            StopCause::Failure(_) => self.keep.on_failure,
        };
        if !chosen {
            let StopCause::Failure(code) = stopped.cause else {
                // Unreachable in practice: a session without the cancel choice refuses CancelTurn.
                return Judged::End(SessionEnd::agent_failed("turn_boundary_unverified"));
            };
            return Judged::End(SessionEnd::agent_failed(code));
        }
        if !self.boundary_verified {
            return Judged::End(SessionEnd::agent_failed("turn_boundary_unverified"));
        }
        if !self.mirror.fence().turn_settled() {
            return Judged::End(SessionEnd::agent_failed("turn_not_quiescent"));
        }
        let quiet = tokio::time::sleep(self.turn_cancel.quiet);
        tokio::pin!(quiet);
        loop {
            tokio::select! {
                () = &mut quiet => break,
                () = self.wake.notified() => {
                    match self.queued_end() {
                        Ok(Some(end)) => return Judged::EndRequested(end),
                        Ok(None) => {}
                        Err(end) => return Judged::End(end),
                    }
                    if self.mirror.is_broken() {
                        return Judged::End(SessionEnd::agent_failed("thread_unavailable"));
                    }
                    match self.settle_cancels(Some(&stopped.turn_id)) {
                        Ok(cancels) => merge(&mut stopped.merged, cancels),
                        Err(end) => return Judged::End(end),
                    }
                }
            }
        }
        match self
            .manager
            .take_idle_turn_traffic(self.session_id.as_ref())
            .await
        {
            Ok(false) => {}
            Ok(true) | Err(_) => {
                return Judged::End(SessionEnd::agent_failed("turn_not_quiescent"));
            }
        }
        self.mirror.fence().fence_turn();
        // The quiet window may have ended just as a cancel's wake arrived: every cancel of this
        // turn accepted by now merges into this stop and shares its conclusion.
        match self.settle_cancels(Some(&stopped.turn_id)) {
            Ok(cancels) => merge(&mut stopped.merged, cancels),
            Err(end) => return Judged::End(end),
        }
        // A turn stopped by failure already had its own result when a cancel merged into it.
        let outcome = match stopped.cause {
            StopCause::Cancel => TurnCancelOutcome::Stopped,
            StopCause::Failure(_) => TurnCancelOutcome::AlreadyEnded,
        };
        if let Err(end) = self.settle_merged(&stopped.merged, outcome) {
            return Judged::End(end);
        }
        self.turn_ended(&stopped.turn_id);
        Judged::Kept
    }

    /// Settles every cancel merged into one stop with the same outcome, in acceptance order.
    pub(super) fn settle_merged(
        &self,
        merged: &[CommandId],
        outcome: TurnCancelOutcome,
    ) -> Result<(), SessionEnd> {
        for command in merged {
            self.settle_one(command, outcome, None)?;
        }
        Ok(())
    }

    /// Records one settlement; a ledger that cannot write ends the session, unless an
    /// `EndSession` raced in, whose decision then settles the cancel.
    fn settle_one(
        &self,
        command: &CommandId,
        outcome: TurnCancelOutcome,
        target: Option<&CommandId>,
    ) -> Result<TurnCancelOutcome, SessionEnd> {
        match self
            .ledger
            .settle_turn_cancel(self.execution, command, outcome, target)
        {
            Ok(recorded) => {
                forget_acceptance(self.cancel_acceptance, self.execution, command);
                Ok(recorded)
            }
            Err(error) => {
                if self.queued_end()?.is_some() {
                    return Ok(TurnCancelOutcome::SessionEnding);
                }
                ora_warn!(execution_id = %self.execution, command_id = %command, error = %error, "turn cancel was not settled");
                Err(SessionEnd::agent_failed("ledger_unavailable"))
            }
        }
    }

    /// Whether the turn already has its own result in this session.
    fn has_ended(&self, turn: &TurnId) -> bool {
        self.turns.has_ended(turn)
    }

    /// Remembers that the turn has its own result, so a later cancel finds it ended.
    pub(super) fn turn_ended(&self, turn: &TurnId) {
        self.turns.ended(turn);
    }
}

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
    fn has_ended(&self, turn: &TurnId) -> bool {
        self.ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(turn)
    }

    /// Remembers that the turn has its own result.
    fn ended(&self, turn: &TurnId) {
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
fn forget_acceptance(acceptance: &Acceptance, execution: &ExecutionId, command: &CommandId) {
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

/// Adds the cancels not yet merged into a stop, keeping acceptance order.
fn merge(merged: &mut Vec<CommandId>, cancels: Vec<PendingCancel>) {
    for cancel in cancels {
        if !merged.contains(&cancel.command_id) {
            merged.push(cancel.command_id);
        }
    }
}

/// What a cancel of the turn waiting for admission did.
pub(super) enum WaitingCancel {
    None,
    /// Admission committed first; the cancel stays queued for the running turn.
    Started,
    /// The turn was dropped (or had already timed out) and will never run.
    Dropped,
}

/// The per-session keep choices of `AgentSessionSpec`.
#[derive(Clone, Copy, Debug)]
pub(in crate::session) struct Keep {
    pub(in crate::session) on_cancel: bool,
    pub(in crate::session) on_failure: bool,
}

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
                Ok(_) => {}
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
            if merged.is_empty() {
                // The deadline is fixed by the first cancel of this stop and never moves; the
                // runtime keeps the first deadline it is given.
                let deadline = tokio::time::Instant::now() + self.turn_cancel.grace;
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
                        Ok(cancels) => stopped
                            .merged
                            .extend(cancels.into_iter().map(|cancel| cancel.command_id)),
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
            Ok(recorded) => Ok(recorded),
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
        self.ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(turn)
    }

    /// Remembers that the turn has its own result, so a later cancel finds it ended.
    pub(super) fn turn_ended(&self, turn: &TurnId) {
        self.ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(turn.clone());
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

//! Serialized conversation and turn outcomes.
use super::*;
mod admission;
mod cancel;
use agent_client_protocol_schema::v1::StopReason;
pub(super) use cancel::{Keep, TurnBook, settle_before_start};
use ora_node_protocol::TurnCancelOutcome;

/// How one running turn stopped.
enum TurnOutcome {
    Finished,
    /// A cancel dropped the turn before admission; the Agent never received it.
    Dropped,
    /// The Agent answered after a cancel, or answered with a failure: D2 decides what follows.
    Stopped(cancel::StoppedTurn),
    /// A cancel's deadline passed without an answer.
    Unanswered,
    /// An update of a stopped turn arrived in a later one.
    Leaked,
    Failed(&'static str),
    EndRequested {
        command_id: CommandId,
        reason: EndSessionReason,
        discarded: Vec<CommandId>,
    },
    ThreadBroken,
    LedgerUnavailable,
}

/// The conversation of one session execution over its composed runtime.
pub(super) struct Conversation<'a, L: SessionLedger> {
    pub(super) ledger: &'a L,
    pub(super) execution: &'a ExecutionId,
    pub(super) session_id: SessionId,
    pub(super) manager: &'a AgentRuntimeManager<NodeRuntimeHost<L>>,
    pub(super) mirror: &'a ThreadMirror<L>,
    pub(super) wake: &'a Notify,
    pub(super) control: Option<&'a crate::session::mcp::control::Control>,
    pub(super) started: std::sync::atomic::AtomicBool,
    pub(super) blocked: std::sync::Mutex<
        std::collections::HashMap<ora_node_protocol::TurnId, tokio::time::Instant>,
    >,
    /// Attached to the end decision this conversation records before it stops the Agent.
    pub(super) node: ora_node_protocol::NodeRuntimeIdentity,
    pub(super) keep: Keep,
    /// Whether this Agent version has a verified turn boundary (D2).
    pub(super) boundary_verified: bool,
    pub(super) turn_cancel: &'a crate::session::TurnCancelConfig,
    /// Acceptance times of pending cancels, written by the protocol side at durable acceptance.
    pub(super) cancel_acceptance: &'a std::sync::Mutex<
        std::collections::HashMap<(ExecutionId, CommandId), tokio::time::Instant>,
    >,
    /// The stop clock the end decision anchors together with itself.
    pub(super) stop_config: &'a crate::SessionStopConfig,
    /// Turns that already have their own result, so a later cancel of one is `already_ended`,
    /// including those settled while the session prepared.
    pub(super) turns: &'a TurnBook,
}

impl<L: SessionLedger> Conversation<'_, L> {
    /// Opens the session, runs the initial turn, then serves queued commands until one ends it.
    pub(super) async fn run(
        &self,
        agent_ref: &AgentRef,
        spec: AgentSessionSpec,
        ready_timeout: std::time::Duration,
    ) -> SessionEnd {
        // A cancel accepted while the session prepared may already have dropped the first turn.
        let mut next_turn = (!self.turns.initial_dropped()).then_some(spec.initial_turn);
        loop {
            if self.mirror.is_broken() {
                return self
                    .stop(SessionEnd::agent_failed("thread_unavailable"))
                    .await;
            }
            if self.mirror.fence().leaked() {
                return self
                    .stop(SessionEnd::agent_failed("turn_leak_detected"))
                    .await;
            }
            // Between turns every cancel takes effect at once: a queued target is dropped, an
            // ended one is `already_ended`, anything else `unknown`.
            if next_turn.is_none()
                && let Err(end) = self.settle_cancels(None)
            {
                return self.stop(end).await;
            }
            let (turn, command_id) = match next_turn.take() {
                Some(turn) => (turn, None),
                None => match self.ledger.queued_commands(self.execution).map(plan) {
                    Ok(Plan::Turn { command_id, turn }) => (turn, Some(command_id)),
                    Ok(Plan::End {
                        command_id,
                        reason,
                        discarded,
                    }) => return self.end_requested(command_id, reason, discarded).await,
                    Ok(Plan::Wait) => {
                        self.wake.notified().await;
                        continue;
                    }
                    Err(error) => {
                        ora_warn!(execution_id = %self.execution, error = %error, "session queue unreadable");
                        return self
                            .stop(SessionEnd::agent_failed("ledger_unavailable"))
                            .await;
                    }
                },
            };
            let turn_id = turn.turn_id.clone();
            match self
                .run_turn(
                    turn,
                    command_id.as_ref(),
                    agent_ref,
                    &spec.checkout_execution_id,
                    ready_timeout,
                )
                .await
            {
                Ok(TurnOutcome::Finished) => self.turn_ended(&turn_id),
                Ok(TurnOutcome::Dropped) => {}
                Ok(TurnOutcome::Stopped(stopped)) => match self.judge(stopped).await {
                    cancel::Judged::Kept => self.mirror.set_turn(None),
                    cancel::Judged::EndRequested(TurnOutcome::EndRequested {
                        command_id,
                        reason,
                        discarded,
                    }) => return self.end_requested(command_id, reason, discarded).await,
                    cancel::Judged::EndRequested(_) => {
                        return self
                            .stop(SessionEnd::agent_failed("ledger_unavailable"))
                            .await;
                    }
                    cancel::Judged::End(end) => return self.end_or_requested(end).await,
                },
                Ok(TurnOutcome::Unanswered) => {
                    return self
                        .end_or_requested(SessionEnd::agent_failed("turn_cancel_unanswered"))
                        .await;
                }
                Ok(TurnOutcome::Leaked) => {
                    return self
                        .stop(SessionEnd::agent_failed("turn_leak_detected"))
                        .await;
                }
                Ok(TurnOutcome::Failed(code)) => {
                    // A committed EndSession keeps its reason even if its wake-up races the ACP
                    // result. Never replace a durable cancellation with a provider failure.
                    return match self.ledger.queued_commands(self.execution).map(plan) {
                        Ok(Plan::End {
                            command_id,
                            reason,
                            discarded,
                        }) => self.end_requested(command_id, reason, discarded).await,
                        Ok(Plan::Turn { .. } | Plan::Wait) => {
                            self.stop(SessionEnd::agent_failed(code)).await
                        }
                        Err(_) => {
                            self.stop(SessionEnd::agent_failed("ledger_unavailable"))
                                .await
                        }
                    };
                }
                Ok(TurnOutcome::EndRequested {
                    command_id,
                    reason,
                    discarded,
                }) => return self.end_requested(command_id, reason, discarded).await,
                Ok(TurnOutcome::ThreadBroken) => {
                    return self
                        .stop(SessionEnd::agent_failed("thread_unavailable"))
                        .await;
                }
                Ok(TurnOutcome::LedgerUnavailable) => {
                    return self
                        .stop(SessionEnd::agent_failed("ledger_unavailable"))
                        .await;
                }
                Err(end) => return self.end_or_requested(end).await,
            }
        }
    }

    /// Ends with `end`, unless a committed `EndSession` is queued: its reason wins.
    async fn end_or_requested(&self, end: SessionEnd) -> SessionEnd {
        match self.queued_end() {
            Ok(Some(TurnOutcome::EndRequested {
                command_id,
                reason,
                discarded,
            })) => self.end_requested(command_id, reason, discarded).await,
            _ => self.stop(end).await,
        }
    }

    /// Runs one user turn until the agent completes it or an `EndSession` interrupts it.
    ///
    /// Records settled while the turn runs, the cancellation an end causes included, belong to
    /// it; the attribution is cleared only once the turn can produce nothing more.
    async fn run_turn(
        &self,
        turn: UserTurn,
        command_id: Option<&CommandId>,
        agent_ref: &AgentRef,
        checkout: &ExecutionId,
        ready_timeout: std::time::Duration,
    ) -> Result<TurnOutcome, SessionEnd> {
        let mut stream = match self
            .admit_turn(&turn, command_id, agent_ref, checkout, ready_timeout)
            .await?
        {
            admission::Admitted::Stream(stream) => stream,
            admission::Admitted::Outcome(outcome) => return Ok(outcome),
        };
        // Business time belongs to the admitted turn, never to queued MCP wait budgets.
        let _business = admission::BusinessTime::new(&self.blocked);
        let turn_id = turn.turn_id.clone();
        // Cancels merged into this turn's stop, in acceptance order. Cancels accepted while the
        // turn was being admitted take effect now that it has started.
        let mut merged = Vec::new();
        self.request_stop(&mut merged, self.settle_cancels(Some(&turn_id))?);
        let outcome = loop {
            tokio::select! {
                event = stream.recv() => match event {
                    Some(Ok(PromptSessionEvent::Completed { stop_reason, .. })) => {
                        if merged.is_empty() {
                            break TurnOutcome::Finished;
                        }
                        if stream.end_report().is_some_and(|report| report.unsent()) {
                            // Admitted but never written: nothing to judge, the session stays.
                            self.settle_merged(&merged, TurnCancelOutcome::Stopped)?;
                            break TurnOutcome::Finished;
                        }
                        if stop_reason == StopReason::Cancelled {
                            break TurnOutcome::Stopped(cancel::StoppedTurn {
                                turn_id: turn_id.clone(),
                                cause: cancel::StopCause::Cancel,
                                merged: std::mem::take(&mut merged),
                            });
                        }
                        // The Agent finished the turn before the cancel reached it: the turn keeps
                        // its own end and the cancel did nothing.
                        self.settle_merged(&merged, TurnCancelOutcome::AlreadyEnded)?;
                        break TurnOutcome::Finished;
                    }
                    Some(Err(error)) => {
                        if !merged.is_empty() && error.is_turn_cancel_unanswered() {
                            break TurnOutcome::Unanswered;
                        }
                        let code = match error.public_error() {
                            PublicError::AgentAuthenticationRequired(_) => {
                                "agent_authentication_required"
                            }
                            _ => "agent_turn_failed",
                        };
                        if error.agent_answered() {
                            break TurnOutcome::Stopped(cancel::StoppedTurn {
                                turn_id: turn_id.clone(),
                                cause: cancel::StopCause::Failure(code),
                                merged: std::mem::take(&mut merged),
                            });
                        }
                        break TurnOutcome::Failed(code);
                    }
                    None => break TurnOutcome::Failed("agent_stream_closed"),
                    Some(Ok(_)) => {}
                },
                () = self.wake.notified() => {
                    if self.mirror.is_broken() {
                        break TurnOutcome::ThreadBroken;
                    }
                    if self.mirror.fence().leaked() {
                        break TurnOutcome::Leaked;
                    }
                    match self.ledger.queued_commands(self.execution).map(plan) {
                        Ok(Plan::End { command_id, reason, discarded }) => {
                            break TurnOutcome::EndRequested { command_id, reason, discarded };
                        }
                        Ok(Plan::Turn { .. } | Plan::Wait) => {}
                        Err(error) => {
                            ora_warn!(execution_id = %self.execution, error = %error, "session queue unreadable");
                            break TurnOutcome::LedgerUnavailable;
                        }
                    }
                    match self.settle_cancels(Some(&turn_id)) {
                        Ok(cancels) => self.request_stop(&mut merged, cancels),
                        Err(_) => break TurnOutcome::LedgerUnavailable,
                    }
                }
            }
        };
        match outcome {
            TurnOutcome::Finished => self.mirror.set_turn(None),
            // The turn is still running: stopping cancels it and records its end under this turn.
            TurnOutcome::EndRequested { .. }
            | TurnOutcome::Failed(_)
            | TurnOutcome::ThreadBroken
            | TurnOutcome::LedgerUnavailable
            | TurnOutcome::Unanswered
            | TurnOutcome::Leaked
            | TurnOutcome::Stopped(_)
            | TurnOutcome::Dropped => {}
        }
        drop(stream);
        Ok(outcome)
    }

    /// Ends the session as a command asked: queued turns ahead of it never run.
    async fn end_requested(
        &self,
        command_id: CommandId,
        reason: EndSessionReason,
        discarded: Vec<CommandId>,
    ) -> SessionEnd {
        for discarded in &discarded {
            settle(
                self.ledger,
                self.execution,
                discarded,
                CommandSettlement::Discarded,
            );
        }
        let end = self.stop(SessionEnd::requested(reason)).await;
        settle(
            self.ledger,
            self.execution,
            &command_id,
            CommandSettlement::Executed,
        );
        end
    }

    /// Stops the session, cancelling a running turn and closing the provider session.
    ///
    /// The end decision is recorded before the Agent is signalled: its transaction also settles
    /// every pending cancel as `session_ending`, so a visible `session_ending` always has a
    /// recoverable decision (D3). A ledger that cannot write still lets the stop proceed, because
    /// stopping is always safe; the decision is written by the driver or after recovery.
    pub(super) async fn stop(&self, end: SessionEnd) -> SessionEnd {
        if let Err(error) = crate::session::stop::decide(
            self.ledger,
            self.execution,
            &end.terminal(self.node.clone()),
            self.stop_config,
        ) {
            ora_warn!(execution_id = %self.execution, error = %error, "session end decision was not recorded before stopping");
        }
        self.stop_undecided().await;
        end
    }

    /// Stops the Agent without recording a decision, for an end whose reason is still unknown;
    /// the caller records the decision once the reason is settled, before the terminal.
    pub(super) async fn stop_undecided(&self) {
        if let Some(control) = self.control {
            control.stop();
        }
        if let Err(error) = self
            .manager
            .stop_session(StopSessionRequest {
                session_id: self.session_id.to_string(),
            })
            .await
        {
            ora_warn!(execution_id = %self.execution, error = %error, "agent session did not stop cleanly");
        }
        self.mirror.set_turn(None);
    }
}

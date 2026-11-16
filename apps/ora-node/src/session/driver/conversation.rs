//! Serialized conversation and turn outcomes.
use super::*;

/// How one running turn stopped.
enum TurnOutcome {
    Finished,
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
}

impl<L: SessionLedger> Conversation<'_, L> {
    /// Opens the session, runs the initial turn, then serves queued commands until one ends it.
    pub(super) async fn run(
        &self,
        agent_ref: &AgentRef,
        spec: AgentSessionSpec,
        ready_timeout: std::time::Duration,
    ) -> SessionEnd {
        match tokio::time::timeout(ready_timeout, self.manager.wait_for_agent(agent_ref)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                ora_warn!(execution_id = %self.execution, error = %error, "agent did not become ready");
                return SessionEnd::agent_failed("agent_unavailable");
            }
            Err(_) => return SessionEnd::agent_failed("agent_unavailable"),
        }
        if let Err(error) = self
            .manager
            .start_session_with_id(
                self.session_id.clone(),
                StartSessionRequest {
                    workspace_id: spec.checkout_execution_id.to_string(),
                    agent_ref: agent_ref.to_string(),
                    model: None,
                },
            )
            .await
        {
            ora_warn!(execution_id = %self.execution, error = %error, "agent session did not start");
            return crate::session::mcp::start_failure(&error, self.execution);
        }

        let mut next_turn = Some(spec.initial_turn);
        loop {
            if self.mirror.is_broken() {
                return self
                    .stop(SessionEnd::agent_failed("thread_unavailable"))
                    .await;
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
            match self.run_turn(turn, command_id.as_ref()).await {
                Ok(TurnOutcome::Finished) => {}
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
                Err(end) => return self.stop(end).await,
            }
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
    ) -> Result<TurnOutcome, SessionEnd> {
        self.mirror.set_turn(Some(turn.turn_id.clone()));
        let ledger = self.ledger.clone();
        let execution = self.execution.clone();
        let command_id = command_id.cloned();
        let admission_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let commit_failed = Arc::clone(&admission_failed);
        let prompt = turn
            .content
            .into_iter()
            .map(|block| match block {
                ContentBlock::Text { text } => AcpContentBlock::Text(TextContent::new(text)),
            })
            .collect();
        let stream = self
            .manager
            .prompt_session_with_admission(
                PromptSessionRequest {
                    session_id: self.session_id.to_string(),
                    prompt,
                    model: None,
                    record_prompt: None,
                },
                MessageId::new(turn.turn_id.as_str()),
                move || {
                    if let Some(command_id) = command_id {
                        ledger
                            .settle_command(&execution, &command_id, CommandSettlement::Executed)
                            .map_err(|error| {
                                commit_failed.store(true, std::sync::atomic::Ordering::Release);
                                ora_agent_runtime::RuntimeError::internal(
                                    "command admission could not commit",
                                    error,
                                )
                            })?;
                    }
                    Ok(())
                },
            )
            .await;
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                self.mirror.set_turn(None);
                ora_warn!(execution_id = %self.execution, error = %error, "turn was not admitted");
                return Err(SessionEnd::agent_failed(
                    if admission_failed.load(std::sync::atomic::Ordering::Acquire) {
                        "ledger_unavailable"
                    } else {
                        "agent_unavailable"
                    },
                ));
            }
        };
        let outcome = loop {
            tokio::select! {
                event = stream.recv() => match event {
                    Some(Ok(PromptSessionEvent::Completed { .. })) => break TurnOutcome::Finished,
                    Some(Err(error)) => {
                        let code = match error.public_error() {
                            PublicError::AgentAuthenticationRequired(_) => {
                                "agent_authentication_required"
                            }
                            _ => "agent_turn_failed",
                        };
                        break TurnOutcome::Failed(code);
                    }
                    None => break TurnOutcome::Failed("agent_stream_closed"),
                    Some(Ok(_)) => {}
                },
                () = self.wake.notified() => {
                    if self.mirror.is_broken() {
                        break TurnOutcome::ThreadBroken;
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
                }
            }
        };
        match outcome {
            TurnOutcome::Finished => self.mirror.set_turn(None),
            // The turn is still running: stopping cancels it and records its end under this turn.
            TurnOutcome::EndRequested { .. }
            | TurnOutcome::Failed(_)
            | TurnOutcome::ThreadBroken
            | TurnOutcome::LedgerUnavailable => {}
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
    pub(super) async fn stop(&self, end: SessionEnd) -> SessionEnd {
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
        end
    }
}

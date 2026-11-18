//! Turn-local bounded waits share the update owner's final admission lock.
use super::*;
use ora_agent_runtime::{RuntimeError, SessionEventStream};
use ora_node_protocol::{SessionMcpConvergence, SessionTurnFailed, SessionTurnFailureCode};
use std::sync::atomic::Ordering;
use tokio::time::Instant;
#[cfg(test)]
mod tests;

pub(super) enum Admitted {
    Stream(SessionEventStream<PromptSessionEvent>),
    Outcome(TurnOutcome),
}

/// Cancellation and the Actor's commit share a boundary even before a reply reaches the caller.
#[derive(Default)]
struct TurnGate {
    committed: bool,
    expired: bool,
    deadline: Option<Instant>,
}

impl TurnGate {
    /// Final Actor commit and timeout share this lock; a committed turn cannot later time out.
    fn commit(
        &mut self,
        write: impl FnOnce() -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if self.expired
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(RuntimeError::internal(
                "MCP admission deadline expired",
                crate::session::mcp::control::Admission::Pending,
            ));
        }
        write()?;
        self.committed = true;
        Ok(())
    }

    /// Returns whether timeout won against the Actor's final commit.
    fn expire(&mut self) -> bool {
        if self.committed {
            return false;
        }
        self.expired = true;
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("agent readiness unavailable")]
struct ReadinessUnavailable;

impl<L: SessionLedger> Conversation<'_, L> {
    /// Waiting turns share MCP blocking time, excluding a previously admitted turn's business time.
    fn expire_queued(
        &self,
        current: &ora_node_protocol::TurnId,
    ) -> Result<Option<Instant>, SessionEnd> {
        let commands = self
            .ledger
            .queued_commands(self.execution)
            .map_err(|_| SessionEnd::agent_failed("ledger_unavailable"))?;
        if commands.iter().any(|command| {
            matches!(
                command.command,
                crate::session::SessionCommand::EndSession { .. }
            )
        }) {
            return Ok(None);
        }
        let mut next = None;
        for command in commands {
            let crate::session::SessionCommand::SubmitUserTurn(turn) = command.command else {
                continue;
            };
            if &turn.turn_id == current {
                continue;
            }
            let deadline = self
                .blocked
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&turn.turn_id)
                .map(|first| *first + std::time::Duration::from_secs(300));
            let Some(deadline) = deadline else {
                continue;
            };
            if Instant::now() < deadline {
                next = Some(next.map_or(deadline, |previous: Instant| previous.min(deadline)));
                continue;
            }
            if self
                .ledger
                .fail_session_turn(
                    self.execution,
                    SessionTurnFailed {
                        turn_id: turn.turn_id.clone(),
                        command_id: Some(command.command_id),
                        code: SessionTurnFailureCode::McpRefreshWaitTimeout,
                    },
                )
                .is_err()
            {
                if self.queued_end()?.is_some() {
                    return Ok(None);
                }
                return Err(SessionEnd::agent_failed("ledger_unavailable"));
            }
            self.blocked
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&turn.turn_id);
        }
        Ok(next)
    }
    /// Registers every MCP-blocked queued turn once, independent of queue head or target changes.
    fn deadline(&self, turn: &UserTurn) -> Result<Option<Instant>, SessionEnd> {
        let blocked = self.control.is_some_and(|control| {
            control.snapshot(true).convergence != SessionMcpConvergence::Current
        });
        let mut times = self
            .blocked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if blocked {
            let now = Instant::now();
            times.entry(turn.turn_id.clone()).or_insert(now);
            for command in self
                .ledger
                .queued_commands(self.execution)
                .map_err(|_| SessionEnd::agent_failed("ledger_unavailable"))?
            {
                if let crate::session::SessionCommand::SubmitUserTurn(turn) = command.command {
                    times.entry(turn.turn_id).or_insert(now);
                }
            }
        }
        Ok(times
            .get(&turn.turn_id)
            .map(|first| *first + std::time::Duration::from_secs(300)))
    }

    /// EndSession is checked before attempts, after wakes, and again inside the durable commit.
    pub(super) fn queued_end(&self) -> Result<Option<TurnOutcome>, SessionEnd> {
        Ok(
            match self
                .ledger
                .queued_commands(self.execution)
                .map(plan)
                .map_err(|_| SessionEnd::agent_failed("ledger_unavailable"))?
            {
                Plan::End {
                    command_id,
                    reason,
                    discarded,
                } => Some(TurnOutcome::EndRequested {
                    command_id,
                    reason,
                    discarded,
                }),
                Plan::Turn { .. } | Plan::Wait => None,
            },
        )
    }

    /// A single attempt per turn/generation may attach or refresh; further waits require new input.
    pub(super) async fn admit_turn(
        &self,
        turn: &UserTurn,
        command: Option<&CommandId>,
        agent: &AgentRef,
        checkout: &ExecutionId,
        ready_timeout: std::time::Duration,
    ) -> Result<Admitted, SessionEnd> {
        let mut attempted = None;
        loop {
            if let Some(end) = self.queued_end()? {
                return Ok(Admitted::Outcome(end));
            }
            let deadline = self.deadline(turn)?;
            self.expire_queued(&turn.turn_id)?;
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                self.mirror.set_turn(None);
                let result = self.ledger.fail_session_turn(
                    self.execution,
                    SessionTurnFailed {
                        turn_id: turn.turn_id.clone(),
                        command_id: command.cloned(),
                        code: SessionTurnFailureCode::McpRefreshWaitTimeout,
                    },
                );
                if result.is_err() {
                    if let Some(end) = self.queued_end()? {
                        return Ok(Admitted::Outcome(end));
                    }
                    return Err(SessionEnd::agent_failed("ledger_unavailable"));
                }
                self.blocked
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&turn.turn_id);
                return Ok(Admitted::Outcome(TurnOutcome::Finished));
            }
            let generation = self
                .control
                .map(|control| control.snapshot(true).desired.generation);
            // A newly queued turn supplies one retry; notifications alone never supply retries.
            let retry = self
                .ledger
                .queued_commands(self.execution)
                .map_err(|_| SessionEnd::agent_failed("ledger_unavailable"))?
                .last()
                .map(|command| command.command_id.clone());
            let key = (generation, retry);
            let current = self.control.is_some_and(|control| {
                control.snapshot(true).convergence == SessionMcpConvergence::Current
            });
            if self.control.is_none() || attempted.as_ref() != Some(&key) || current {
                attempted = Some(key);
                let gate = Arc::new(std::sync::Mutex::new(TurnGate::default()));
                let attempt = self.attempt_turn(
                    turn,
                    command,
                    agent,
                    checkout,
                    ready_timeout,
                    Arc::clone(&gate),
                );
                tokio::pin!(attempt);
                loop {
                    let deadline = self.deadline(turn)?;
                    let deadline = {
                        let mut gate = gate
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        gate.deadline = deadline;
                        if gate.committed { None } else { deadline }
                    };
                    tokio::select! {
                        biased;
                        () = self.wake.notified() => { if let Some(end) = self.queued_end()? { return Ok(Admitted::Outcome(end)); } },
                        result = &mut attempt => {
                            match result {
                                Ok(stream) => { self.blocked.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&turn.turn_id); return Ok(Admitted::Stream(stream)); },
                                Err(error) if self.control.is_some() && mcp_wait(&error.0) => {
                                    if error.1 {
                                        if let Some(end) = self.queued_end()? { return Ok(Admitted::Outcome(end)); }
                                        return Err(SessionEnd::agent_failed("ledger_unavailable"));
                                    }
                                    self.mirror.set_turn(None);
                                    // Only the resolver/load owner publishes failure. Busy and update
                                    // races must not clear another request's in-flight generation.
                                    break;
                                },
                                Err((error, failed)) => { return Err(if failed { SessionEnd::agent_failed("ledger_unavailable") } else if let Some(end) = crate::session::mcp::host_start_failure(&error, self.execution) { end } else if self.control.is_some() || std::error::Error::source(&error).is_some_and(<dyn std::error::Error + 'static>::is::<ReadinessUnavailable>) { SessionEnd::agent_failed("agent_unavailable") } else { crate::session::mcp::start_failure(&error, self.execution) }); },
                            }
                        },
                        () = changed(self.control) => {},
                        () = until(deadline) => {
                            let mut gate = gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                            if gate.expire() { break; }
                        },
                    }
                }
                continue;
            }
            tokio::select! {
                () = self.wake.notified() => {},
                () = changed(self.control) => {},
                () = until(deadline) => {},
            }
        }
    }

    /// The callback is executed by the Actor only after readiness and before any prompt record/send.
    async fn attempt_turn(
        &self,
        turn: &UserTurn,
        command: Option<&CommandId>,
        agent: &AgentRef,
        checkout: &ExecutionId,
        ready_timeout: std::time::Duration,
        gate: Arc<std::sync::Mutex<TurnGate>>,
    ) -> Result<SessionEventStream<PromptSessionEvent>, (RuntimeError, bool)> {
        if !self.started.load(Ordering::Acquire) {
            tokio::time::timeout(ready_timeout, self.manager.wait_for_agent(agent))
                .await
                .map_err(|_error| {
                    (
                        RuntimeError::internal("agent readiness timed out", ReadinessUnavailable),
                        false,
                    )
                })?
                .map_err(|error| {
                    (
                        if mcp_wait(&error) {
                            error
                        } else {
                            RuntimeError::internal("agent readiness failed", ReadinessUnavailable)
                        },
                        false,
                    )
                })?;
            self.manager
                .start_session_with_id(
                    self.session_id.clone(),
                    StartSessionRequest {
                        workspace_id: checkout.to_string(),
                        agent_ref: agent.to_string(),
                        model: None,
                    },
                )
                .await
                .map_err(|error| (error, false))?;
            self.started.store(true, Ordering::Release);
        }
        let ledger = self.ledger.clone();
        let execution = self.execution.clone();
        let command = command.cloned();
        let control = self.control.cloned();
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let commit_failed = Arc::clone(&failed);
        self.mirror.set_turn(Some(turn.turn_id.clone()));
        let result = self
            .manager
            .prompt_session_with_admission(
                PromptSessionRequest {
                    session_id: self.session_id.to_string(),
                    prompt: turn
                        .content
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Text { text } => {
                                AcpContentBlock::Text(TextContent::new(text.clone()))
                            }
                        })
                        .collect(),
                    model: None,
                    record_prompt: None,
                },
                MessageId::new(turn.turn_id.as_str()),
                move || {
                    let commit = || {
                        let mut gate = gate
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        gate.commit(|| {
                            if let Some(command) = command {
                                ledger
                                    .settle_command(
                                        &execution,
                                        &command,
                                        CommandSettlement::Executed,
                                    )
                                    .map_err(|error| {
                                        commit_failed.store(true, Ordering::Release);
                                        RuntimeError::internal(
                                            "command admission could not commit",
                                            error,
                                        )
                                    })?;
                            }
                            Ok(())
                        })
                    };
                    match control {
                        Some(control) => control.admit(commit).map_err(|error| {
                            RuntimeError::internal("MCP admission changed", error)
                        })?,
                        None => commit(),
                    }
                },
            )
            .await;
        result.map_err(|error| (error, failed.load(Ordering::Acquire)))
    }
}

/// Only typed MCP failures and an in-flight refresh can keep a turn waiting.
fn mcp_wait(error: &RuntimeError) -> bool {
    error.is_mcp_admission()
        || matches!(error.public_error(), PublicError::SessionBusy(_))
        || std::error::Error::source(error).is_some_and(|source| {
            source.is::<crate::session::mcp::NodeMcpFailure>()
                || source.is::<crate::session::mcp::control::Admission>()
        })
}

/// Shifts retained wait origins on every exit path, including cancellation and terminal errors.
pub(super) struct BusinessTime<'a> {
    clocks: &'a std::sync::Mutex<std::collections::HashMap<ora_node_protocol::TurnId, Instant>>,
    started: Instant,
}

impl<'a> BusinessTime<'a> {
    pub(super) fn new(
        clocks: &'a std::sync::Mutex<std::collections::HashMap<ora_node_protocol::TurnId, Instant>>,
    ) -> Self {
        Self {
            clocks,
            started: Instant::now(),
        }
    }
}

impl Drop for BusinessTime<'_> {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        for first in self
            .clocks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values_mut()
        {
            *first += elapsed;
        }
    }
}

/// Notifications carry no settings and readers always recheck state before acting.
async fn changed(control: Option<&crate::session::mcp::control::Control>) {
    match control {
        Some(control) => control.changed.notified().await,
        None => std::future::pending().await,
    }
}

/// Ordinary queue wait does not start an MCP timeout.
pub(super) async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

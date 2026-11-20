//! Application-owned adapters keep the database independent of the session runtime.
use super::{CheckoutResolver, CommandSettlement, QueuedCommand, SessionCommand, SessionLedger};
use ora_node_db::{SessionCommandInput, SessionCommandSettlement, SessionJournal, WriteGuard};
use ora_node_protocol::{AgentSessionEnded, CommandId, ExecutionId, Sequence, ThreadEvent};

impl<G: WriteGuard + Send + Sync + 'static> SessionLedger for SessionJournal<G> {
    fn fail_session_turn(
        &self,
        execution: &ExecutionId,
        failure: ora_node_protocol::SessionTurnFailed,
    ) -> Result<(), Self::Error> {
        SessionJournal::fail_session_turn(self, execution, failure).map(drop)
    }
    type Error = ora_node_db::Error;

    fn record_session_stop(
        &self,
        identity: &ora_node_db::SessionStopIdentity,
        scope: ora_node_db::SessionStopScope,
        at_ms: i64,
    ) -> Result<(), Self::Error> {
        SessionJournal::record_session_stop(self, identity, scope, at_ms)
    }

    fn session_stop_records(
        &self,
        execution: Option<&ExecutionId>,
    ) -> Result<Vec<ora_node_db::SessionStopRecord>, Self::Error> {
        SessionJournal::session_stop_records(self, execution)
    }

    fn change_session_stop(
        &self,
        attempt: &ora_node_db::AttemptId,
        change: ora_node_db::SessionStopChange,
    ) -> Result<(), Self::Error> {
        SessionJournal::change_session_stop(self, attempt, change)
    }

    /// One transaction for the decision, the stop clock and the cancel settlements.
    fn decide_session_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
        at_ms: i64,
        bound_at_ms: i64,
    ) -> Result<(), Self::Error> {
        SessionJournal::decide_session_end(self, execution, ended, at_ms, bound_at_ms)
    }

    /// History release follows durable terminal evidence, never actor lifetime alone.
    fn session_completed(&self, execution: &ExecutionId) -> Result<bool, Self::Error> {
        SessionJournal::session_completed(self, execution)
    }

    /// Retains the intended original terminal while stop proof is pending.
    fn defer_restore_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
    ) -> Result<(), Self::Error> {
        SessionJournal::defer_restore_end(self, execution, ended)
    }
    /// Persists the restore boundary before any plugin can be admitted.
    fn restored(&self, execution: &ExecutionId) -> Result<(), Self::Error> {
        SessionJournal::restored(self, execution)
    }
    /// An admission fact survives a later Node crash.
    fn agent_admitted(&self, execution: &ExecutionId) -> Result<(), Self::Error> {
        SessionJournal::agent_admitted(self, execution)
    }

    /// Allocates the sequence only after the database commits the event.
    fn append_thread_event(
        &self,
        execution: &ExecutionId,
        event: ThreadEvent,
    ) -> Result<Sequence, Self::Error> {
        SessionJournal::append_thread_event(self, execution, event)
    }

    /// Commits terminal evidence and discards the remaining queue together.
    fn end_session(
        &self,
        execution: &ExecutionId,
        ended: AgentSessionEnded,
    ) -> Result<Sequence, Self::Error> {
        SessionJournal::end_session(self, execution, ended)
    }

    /// Adapts persisted envelopes without changing their acceptance order.
    fn queued_commands(&self, execution: &ExecutionId) -> Result<Vec<QueuedCommand>, Self::Error> {
        Ok(SessionJournal::queued_commands(self, execution)?
            .into_iter()
            .map(|input| {
                let command_id = input.command_id().clone();
                let command = match input {
                    SessionCommandInput::SubmitUserTurn(message) => {
                        SessionCommand::SubmitUserTurn(message.payload.turn)
                    }
                    SessionCommandInput::EndSession(message) => {
                        SessionCommand::EndSession(message.payload.reason)
                    }
                    SessionCommandInput::CancelTurn(message) => {
                        SessionCommand::CancelTurn(message.payload.turn_id)
                    }
                };
                QueuedCommand {
                    command_id,
                    command,
                }
            })
            .collect())
    }

    /// Delegates to the ledger, which is the serial boundary for cancel, admission and timeout.
    fn settle_turn_cancel(
        &self,
        execution: &ExecutionId,
        command_id: &CommandId,
        outcome: ora_node_protocol::TurnCancelOutcome,
        target: Option<&CommandId>,
    ) -> Result<ora_node_protocol::TurnCancelOutcome, Self::Error> {
        SessionJournal::settle_turn_cancel(self, execution, command_id, outcome, target)
            .map(|(outcome, _)| outcome)
    }

    /// Keeps runtime settlements within the database's one-way command state machine.
    fn settle_command(
        &self,
        execution: &ExecutionId,
        command_id: &CommandId,
        settlement: CommandSettlement,
    ) -> Result<(), Self::Error> {
        let settlement = match settlement {
            CommandSettlement::Executed => SessionCommandSettlement::Executed,
            CommandSettlement::Discarded => SessionCommandSettlement::Discarded,
        };
        SessionJournal::settle_command(self, execution, command_id, settlement)
    }
}

impl<G: WriteGuard + Send + Sync + 'static> CheckoutResolver for SessionJournal<G> {
    /// Storage errors fail closed: no unproven checkout can start an agent.
    fn checkout(&self, execution: &ExecutionId) -> Option<ora_node_db::ClonedCheckout> {
        SessionJournal::cloned_checkout(self, execution)
            .ok()
            .flatten()
    }
}

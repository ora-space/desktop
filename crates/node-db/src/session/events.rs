//! Events and terminal state share one transaction and a sequence counter independent of the outbox.
use super::*;

/// The terminal variant also settles queued commands, so no post-terminal write can race it.
enum Record {
    Thread(ThreadEvent),
    TurnFailed(SessionTurnFailed),
    Ended(AgentSessionEnded),
}

impl<G: WriteGuard> SessionJournal<G> {
    /// Atomically records failure and settles the command; EndSession wins at the same write boundary.
    pub fn fail_session_turn(
        &self,
        execution: &ExecutionId,
        failure: SessionTurnFailed,
    ) -> Result<Sequence, Error> {
        self.record_event(execution, Record::TurnFailed(failure))
    }
    /// Persists the next event before exposing its sequence to the transport owner.
    pub fn append_thread_event(
        &self,
        execution: &ExecutionId,
        event: ThreadEvent,
    ) -> Result<Sequence, Error> {
        self.record_event(execution, Record::Thread(event))
    }

    /// Atomically ends the session, emits its last event, and discards every remaining command.
    /// Repeated terminal writes fail; recovery must query the retained terminal state instead.
    pub fn end_session(
        &self,
        execution: &ExecutionId,
        ended: AgentSessionEnded,
    ) -> Result<Sequence, Error> {
        self.record_event(execution, Record::Ended(ended))
    }

    /// An immediate transaction serializes sequence allocation across independent actor handles.
    fn record_event(&self, execution: &ExecutionId, record: Record) -> Result<Sequence, Error> {
        let mut db = self.lock()?;
        let guard = Arc::clone(&db.guard);
        let node = db.node_id.clone();
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = read(&tx, execution)?.ok_or(Error::IdentityConflict)?;
        if matches!(current.state, ExecutionState::Completed(_)) {
            return Err(Error::InvalidTransition);
        }
        let sequence = current
            .last_sequence
            .checked_add(1)
            .filter(|s| *s <= i64::MAX as u64)
            .ok_or(Error::InvalidTransition)?;
        let mut sequence = Sequence::new(sequence);
        let event = match record {
            Record::TurnFailed(payload) => {
                if current.state != ExecutionState::Running {
                    return Err(Error::InvalidTransition);
                }
                let previous: Option<(String, i64)> = tx.query_row("SELECT failure,sequence FROM session_turn_failures WHERE execution=?1 AND turn_id=?2", params![execution.as_str(), payload.turn_id.as_str()], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
                if let Some((previous, sequence)) = previous {
                    return if serde_json::from_str::<SessionTurnFailed>(&previous)? == payload {
                        Ok(Sequence::new(sequence as u64))
                    } else {
                        Err(Error::IdentityConflict)
                    };
                }
                let commands = tx
                    .prepare("SELECT input FROM session_commands WHERE execution=?1")?
                    .query_map([execution.as_str()], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                for command in commands {
                    if matches!(
                        serde_json::from_str::<SessionCommandInput>(&command)?,
                        SessionCommandInput::EndSession(_)
                    ) {
                        return Err(Error::InvalidTransition);
                    }
                }
                guard.before_write(WritePoint::Progress)?;
                if let Some(command) = &payload.command_id {
                    let input: Option<String> = tx.query_row("SELECT input FROM session_commands WHERE execution=?1 AND command_id=?2 AND state='queued'",params![execution.as_str(),command.as_str()],|row|row.get(0)).optional()?;
                    let Some(input) = input else {
                        return Err(Error::InvalidTransition);
                    };
                    if !matches!(serde_json::from_str::<SessionCommandInput>(&input)?, SessionCommandInput::SubmitUserTurn(input) if input.payload.turn.turn_id == payload.turn_id)
                    {
                        return Err(Error::IdentityConflict);
                    }
                    tx.execute("UPDATE session_commands SET state='failed' WHERE execution=?1 AND command_id=?2",params![execution.as_str(),command.as_str()])?;
                } else if payload.turn_id != current.command.payload.spec.initial_turn.turn_id {
                    return Err(Error::IdentityConflict);
                } else if super::turn_cancel::discarded_turn(&tx, execution, &payload.turn_id)? {
                    // A cancel that committed first dropped the initial turn; it has no failure.
                    return Err(Error::InvalidTransition);
                }
                tx.execute(
                    "INSERT INTO session_turn_failures VALUES(?1,?2,?3,?4)",
                    params![
                        execution.as_str(),
                        payload.turn_id.as_str(),
                        serde_json::to_string(&payload)?,
                        sequence.value() as i64
                    ],
                )?;
                NodeToControllerMessage::SessionTurnFailed(SessionTurnFailedMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: current.command.operation_id,
                    execution_id: execution.clone(),
                    sequence,
                    payload,
                })
            }
            Record::Thread(payload) => {
                if current.state != ExecutionState::Running {
                    return Err(Error::InvalidTransition);
                }
                guard.before_write(WritePoint::Progress)?;
                // The pending conclusion of a failed turn outlives the event's acknowledgement (D6).
                super::turn_failure::record_pending_failure(&tx, &current, &payload, sequence)?;
                NodeToControllerMessage::ThreadEvent(ThreadEventMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: current.command.operation_id,
                    execution_id: execution.clone(),
                    sequence,
                    payload,
                })
            }
            Record::Ended(ended) => {
                if crate::session_stop::pending(&tx, execution)? {
                    return Err(Error::ResourceConflict);
                }
                let restore_phase = restore::phase(&tx, execution)?;
                let has_attempts: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM restore_process_attempts WHERE execution=?1)",
                    [execution.as_str()],
                    |row| row.get(/*idx*/ 0),
                )?;
                if restore_phase == Some(RestorePhase::LegacyUnknown)
                    || (matches!(
                        restore_phase,
                        Some(
                            RestorePhase::GitAdmitted
                                | RestorePhase::Restored
                                | RestorePhase::AgentAdmitted
                        )
                    ) && !has_attempts)
                    || !restore::pending(&tx, execution)?.is_empty()
                {
                    return Err(Error::ResourceConflict);
                }
                if ended.node.node_id != node {
                    return Err(Error::NodeMismatch);
                }
                // Every accepted cancel is settled before any terminal (invariant 2), including
                // ends that never went through a deferred decision, such as restart settlement.
                // A failed turn's conclusion precedes the cancels merged into its stop (D6).
                let last = super::settle_failure_as_ending_if_due(
                    &tx,
                    &guard,
                    &current,
                    current.last_sequence,
                )?;
                let last = super::settle_pending_as_ending(&tx, &guard, &current, last)?;
                sequence = last
                    .checked_add(1)
                    .filter(|s| *s <= i64::MAX as u64)
                    .map(Sequence::new)
                    .ok_or(Error::InvalidTransition)?;
                guard.before_write(WritePoint::Complete)?;
                let payload = AgentSessionResult::AgentSessionEnded(ended);
                tx.execute(
                    "UPDATE node_executions SET state='completed',result=?1 WHERE execution=?2",
                    params![serde_json::to_string(&payload)?, execution.as_str()],
                )?;
                tx.execute("UPDATE session_commands SET state='discarded' WHERE execution=?1 AND state='queued'", [execution.as_str()])?;
                NodeToControllerMessage::AgentSessionEnded(AgentSessionEndedMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: current.command.operation_id,
                    execution_id: execution.clone(),
                    sequence,
                    payload,
                })
            }
        };
        event.validate()?;
        tx.execute(
            "UPDATE node_executions SET last_sequence=?1 WHERE execution=?2",
            params![sequence.value() as i64, execution.as_str()],
        )?;
        guard.before_write(WritePoint::Outbox)?;
        tx.execute(
            "INSERT INTO execution_events VALUES(?1,?2,?3)",
            params![
                execution.as_str(),
                sequence.value() as i64,
                serde_json::to_string(&event)?
            ],
        )?;
        tx.commit()?;
        Ok(sequence)
    }
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Deletes exactly one emitted sequence; a repeated ACK is harmless, a future ACK is invalid.
    pub(crate) fn acknowledge_session(&mut self, ack: &EventAckMessage) -> Result<(), Error> {
        let record = self
            .find_session(&ack.operation_id, &ack.execution_id)?
            .ok_or(Error::InvalidAck)?;
        let sequence = ack.sequence.value();
        if sequence == 0 || sequence > record.last_sequence {
            return Err(Error::InvalidAck);
        }
        self.guard.before_write(WritePoint::Acknowledge)?;
        self.connection.execute(
            "DELETE FROM execution_events WHERE execution=?1 AND sequence=?2",
            params![ack.execution_id.as_str(), sequence as i64],
        )?;
        Ok(())
    }
}

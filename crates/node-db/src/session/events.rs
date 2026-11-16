//! Events and terminal state share one transaction and a sequence counter independent of the outbox.
use super::*;

/// The terminal variant also settles queued commands, so no post-terminal write can race it.
enum Record {
    Thread(ThreadEvent),
    Ended(AgentSessionEnded),
}

impl<G: WriteGuard> SessionJournal<G> {
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
        let sequence = Sequence::new(sequence);
        let event = match record {
            Record::Thread(payload) => {
                if current.state != ExecutionState::Running {
                    return Err(Error::InvalidTransition);
                }
                guard.before_write(WritePoint::Progress)?;
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

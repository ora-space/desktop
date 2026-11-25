//! The one durable settlement of a kept-on-failure session's failed turn.
//!
//! A failed turn's `kept` settlement is written in the same transaction as the cancels merged into
//! the stop, so an `EndSession` landing between them can never surface a `kept` that a later
//! `session_ending` contradicts. The ledger refuses `kept` once an `EndSession` was accepted.
use super::*;
use super::turn_cancel::{end_requested, write_settlement};

impl<G: WriteGuard> SessionJournal<G> {
    /// Records `outcome` for one failed turn and, in the same transaction, the settlement of the
    /// cancels merged into the stop. Returns the sequence the failure settlement recorded.
    pub fn settle_turn_failure(
        &self,
        execution: &ExecutionId,
        turn: &TurnId,
        outcome: TurnFailureOutcome,
        merged: &[CommandId],
    ) -> Result<Sequence, Error> {
        let mut db = self.lock()?;
        let guard = Arc::clone(&db.guard);
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = read(&tx, execution)?.ok_or(Error::IdentityConflict)?;
        if current.state != ExecutionState::Running {
            return Err(Error::InvalidTransition);
        }
        if let Some((sequence, recorded)) = failure_settled(&tx, execution, turn)? {
            return if recorded == outcome {
                Ok(sequence)
            } else {
                Err(Error::IdentityConflict)
            };
        }
        // An accepted EndSession wins over every conclusion queued before it (D6): only a
        // `session_ending` settlement may be recorded once one exists.
        if outcome == TurnFailureOutcome::Kept && end_requested(&tx, execution)? {
            return Err(Error::InvalidTransition);
        }
        let mut last = current.last_sequence;
        guard.before_write(WritePoint::Progress)?;
        last = write_failure_settlement(
            &tx,
            &guard,
            &current,
            last,
            TurnFailureSettled {
                turn_id: turn.clone(),
                outcome,
            },
        )?
        .value();
        // A kept failure settles its merged cancels as `already_ended`; an ending one settles them
        // as `session_ending`, so both stay consistent with the failure outcome.
        let cancel_outcome = match outcome {
            TurnFailureOutcome::Kept => TurnCancelOutcome::AlreadyEnded,
            TurnFailureOutcome::SessionEnding => TurnCancelOutcome::SessionEnding,
        };
        for command in merged {
            last = write_settlement(
                &tx,
                &guard,
                &current,
                last,
                TurnCancelSettled {
                    command_id: command.clone(),
                    turn_id: turn.clone(),
                    outcome: cancel_outcome,
                },
            )?
            .value();
        }
        tx.commit()?;
        Ok(Sequence::new(last))
    }

    /// The settlement already recorded for a failed turn, if any.
    pub fn turn_failure_settlement(
        &self,
        execution: &ExecutionId,
        turn: &TurnId,
    ) -> Result<Option<TurnFailureOutcome>, Error> {
        let db = self.lock()?;
        Ok(failure_settled(&db.connection, execution, turn)?.map(|(_, outcome)| outcome))
    }
}

/// Back-fills `TurnFailureSettled{session_ending}` for the last failed turn, when one is due, and
/// returns the execution's new last sequence.
///
/// Called in the transaction of every end decision: the last Thread event was persisted before it,
/// so a keep-on-failure session whose last event is a failed `turnEnded` without a settlement gets
/// its conclusion here, mirroring the cancels the same decision settles as `session_ending` (D6).
pub(crate) fn settle_failure_as_ending_if_due<G: WriteGuard>(
    tx: &rusqlite::Transaction<'_>,
    guard: &Arc<G>,
    current: &SessionExecution,
    last: u64,
) -> Result<u64, Error> {
    if !current.command.payload.spec.keep_session_on_failure {
        return Ok(last);
    }
    let Some(turn) = last_failed_turn(tx, &current.command.execution_id)? else {
        return Ok(last);
    };
    if failure_settled(tx, &current.command.execution_id, &turn)?.is_some() {
        return Ok(last);
    }
    guard.before_write(WritePoint::Progress)?;
    Ok(write_failure_settlement(
        tx,
        guard,
        current,
        last,
        TurnFailureSettled {
            turn_id: turn,
            outcome: TurnFailureOutcome::SessionEnding,
        },
    )?
    .value())
}

/// The failed turn of the last persisted Thread event, when that event is a `turnEnded` carrying a
/// `failure`. A failure without a persisted Thread event has no conclusion to back-fill (D6).
fn last_failed_turn(
    tx: &rusqlite::Transaction<'_>,
    execution: &ExecutionId,
) -> Result<Option<TurnId>, Error> {
    let mut statement =
        tx.prepare("SELECT event FROM execution_events WHERE execution=?1 ORDER BY sequence DESC")?;
    let events = statement.query_map([execution.as_str()], |row| row.get::<_, String>(0))?;
    for event in events {
        let message: NodeToControllerMessage = serde_json::from_str(&event?)?;
        let NodeToControllerMessage::ThreadEvent(thread) = message else {
            continue;
        };
        let Some(turn) = thread.payload.turn_id else {
            continue;
        };
        if thread.payload.record.get("type").and_then(serde_json::Value::as_str) != Some("turnEnded")
        {
            continue;
        }
        if !thread
            .payload
            .record
            .get("failure")
            .is_some_and(|failure| !failure.is_null())
        {
            continue;
        }
        return Ok(Some(turn));
    }
    Ok(None)
}

/// Writes one failed-turn settlement after `last` and queues the event.
fn write_failure_settlement<G: WriteGuard>(
    tx: &rusqlite::Transaction<'_>,
    guard: &Arc<G>,
    current: &SessionExecution,
    last: u64,
    payload: TurnFailureSettled,
) -> Result<Sequence, Error> {
    let execution = &current.command.execution_id;
    let sequence = last
        .checked_add(1)
        .filter(|s| *s <= i64::MAX as u64)
        .map(Sequence::new)
        .ok_or(Error::InvalidTransition)?;
    let event = NodeToControllerMessage::TurnFailureSettled(TurnFailureSettledMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: current.command.operation_id.clone(),
        execution_id: execution.clone(),
        sequence,
        payload,
    });
    event.validate()?;
    let NodeToControllerMessage::TurnFailureSettled(message) = &event else {
        unreachable!("constructed above");
    };
    tx.execute(
        "INSERT INTO session_turn_failure_settlements VALUES(?1,?2,?3,?4)",
        params![
            execution.as_str(),
            message.payload.turn_id.as_str(),
            failure_outcome_token(message.payload.outcome),
            sequence.value() as i64
        ],
    )?;
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
    Ok(sequence)
}

/// The recorded settlement and its sequence for a failed turn.
fn failure_settled(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
    turn: &TurnId,
) -> Result<Option<(Sequence, TurnFailureOutcome)>, Error> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT outcome,sequence FROM session_turn_failure_settlements WHERE execution=?1 AND turn_id=?2",
            params![execution.as_str(), turn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(outcome, sequence)| Ok((Sequence::new(sequence as u64), parse_failure_outcome(&outcome)?)))
        .transpose()
}

/// Stable storage token of a failure outcome; equal to its wire name.
fn failure_outcome_token(outcome: TurnFailureOutcome) -> &'static str {
    match outcome {
        TurnFailureOutcome::Kept => "kept",
        TurnFailureOutcome::SessionEnding => "session_ending",
    }
}

/// Inverse of [`failure_outcome_token`]; anything else means the file was not written by this program.
fn parse_failure_outcome(token: &str) -> Result<TurnFailureOutcome, Error> {
    match token {
        "kept" => Ok(TurnFailureOutcome::Kept),
        "session_ending" => Ok(TurnFailureOutcome::SessionEnding),
        _ => Err(Error::InvalidSchema),
    }
}

//! The one durable settlement of a kept-on-failure session's failed turn.
//!
//! Persisting a failed `turnEnded` Thread event records a pending settlement row in the same
//! transaction. The row is not part of the outbox, so a Controller acknowledging the Thread event
//! cannot remove it: `kept` and every end decision consume that row, and nothing else is read to
//! decide whether a conclusion is due. A failure whose Thread event was never persisted has no row
//! and therefore never receives a conclusion (D6).
//!
//! A failed turn's `kept` settlement is written in the same transaction as the cancels merged into
//! the stop, so an `EndSession` landing between them can never surface a `kept` that a later
//! `session_ending` contradicts. The ledger refuses `kept` once an `EndSession` was accepted.
use super::turn_cancel::{end_requested, write_settlement};
use super::*;

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
        match failure_settled(&tx, execution, turn)? {
            Some(Settlement::Recorded(sequence, recorded)) => {
                return if recorded == outcome {
                    Ok(sequence)
                } else {
                    Err(Error::IdentityConflict)
                };
            }
            Some(Settlement::Pending) => {}
            // No failed `turnEnded` of this turn was persisted, so the Controller never saw the
            // failure and must not be told the session survived it.
            None => return Err(Error::InvalidTransition),
        }
        // An accepted EndSession wins over every conclusion queued before it (D6): only a
        // `session_ending` settlement may be recorded once one exists.
        if outcome == TurnFailureOutcome::Kept && end_requested(&tx, execution)? {
            return Err(Error::InvalidTransition);
        }
        let mut last = current.last_sequence;
        guard.before_write(WritePoint::Progress)?;
        let recorded = write_failure_settlement(
            &tx,
            &guard,
            &current,
            last,
            TurnFailureSettled {
                turn_id: turn.clone(),
                outcome,
            },
        )?;
        last = recorded.value();
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
        Ok(recorded)
    }

    /// The settlement already recorded for a failed turn, if any.
    pub fn turn_failure_settlement(
        &self,
        execution: &ExecutionId,
        turn: &TurnId,
    ) -> Result<Option<TurnFailureOutcome>, Error> {
        let db = self.lock()?;
        Ok(match failure_settled(&db.connection, execution, turn)? {
            Some(Settlement::Recorded(_, outcome)) => Some(outcome),
            Some(Settlement::Pending) | None => None,
        })
    }
}

/// Records the pending settlement of a failed turn whose `turnEnded` Thread event is persisted at
/// `failed_sequence`, inside that event's transaction. Only sessions that chose
/// `keep_session_on_failure` receive conclusions, so only they get a row.
pub(crate) fn record_pending_failure(
    tx: &rusqlite::Transaction<'_>,
    current: &SessionExecution,
    event: &ThreadEvent,
    failed_sequence: Sequence,
) -> Result<(), Error> {
    if !current.command.payload.spec.keep_session_on_failure {
        return Ok(());
    }
    let Some(turn) = &event.turn_id else {
        return Ok(());
    };
    let is_turn_end =
        event.record.get("type").and_then(serde_json::Value::as_str) == Some("turnEnded");
    let failed = event
        .record
        .get("failure")
        .is_some_and(|failure| !failure.is_null());
    if !is_turn_end || !failed {
        return Ok(());
    }
    // A turn ends once; a repeated row would only come from a replayed line and must not reopen
    // a conclusion that was already recorded.
    tx.execute(
        "INSERT OR IGNORE INTO session_turn_failure_settlements(execution,turn_id,failed_sequence) VALUES(?1,?2,?3)",
        params![
            current.command.execution_id.as_str(),
            turn.as_str(),
            failed_sequence.value() as i64
        ],
    )?;
    Ok(())
}

/// Back-fills `TurnFailureSettled{session_ending}` for every failed turn whose settlement is still
/// pending and returns the execution's new last sequence.
///
/// Called in the transaction of every end decision, before the cancels the same decision settles
/// as `session_ending`, so the failure's conclusion precedes the merged cancels' (D6). The pending
/// rows survive acknowledgements and restarts, so a Controller that already acknowledged the
/// failed `turnEnded` still receives its conclusion.
pub(crate) fn settle_failure_as_ending_if_due<G: WriteGuard>(
    tx: &rusqlite::Transaction<'_>,
    guard: &Arc<G>,
    current: &SessionExecution,
    last: u64,
) -> Result<u64, Error> {
    let pending = tx
        .prepare("SELECT turn_id FROM session_turn_failure_settlements WHERE execution=?1 AND outcome IS NULL ORDER BY failed_sequence")?
        .query_map([current.command.execution_id.as_str()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut last = last;
    for turn in pending {
        guard.before_write(WritePoint::Progress)?;
        last = write_failure_settlement(
            tx,
            guard,
            current,
            last,
            TurnFailureSettled {
                turn_id: TurnId::new(turn),
                outcome: TurnFailureOutcome::SessionEnding,
            },
        )?
        .value();
    }
    Ok(last)
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
    // Only a pending row may be concluded: the caller checked it exists in this transaction.
    let concluded = tx.execute(
        "UPDATE session_turn_failure_settlements SET outcome=?3,sequence=?4 WHERE execution=?1 AND turn_id=?2 AND outcome IS NULL",
        params![
            execution.as_str(),
            message.payload.turn_id.as_str(),
            failure_outcome_token(message.payload.outcome),
            sequence.value() as i64
        ],
    )?;
    if concluded != 1 {
        return Err(Error::InvalidTransition);
    }
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

/// The settlement row of one failed turn.
enum Settlement {
    /// The failed `turnEnded` is persisted and its conclusion is still due.
    Pending,
    /// The conclusion and the sequence it was emitted at.
    Recorded(Sequence, TurnFailureOutcome),
}

/// The settlement row of a failed turn; `None` when no failed `turnEnded` of it was persisted.
fn failure_settled(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
    turn: &TurnId,
) -> Result<Option<Settlement>, Error> {
    let row: Option<(Option<String>, Option<i64>)> = connection
        .query_row(
            "SELECT outcome,sequence FROM session_turn_failure_settlements WHERE execution=?1 AND turn_id=?2",
            params![execution.as_str(), turn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|row| match row {
        (None, None) => Ok(Settlement::Pending),
        (Some(outcome), Some(sequence)) => Ok(Settlement::Recorded(
            Sequence::new(sequence as u64),
            parse_failure_outcome(&outcome)?,
        )),
        // The table's CHECK forbids a half-written conclusion.
        (Some(_), None) | (None, Some(_)) => Err(Error::InvalidSchema),
    })
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

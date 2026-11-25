//! The one durable settlement of each accepted `CancelTurn`.
//!
//! The ledger is the serial settlement boundary: a cancel, the target turn's admission and its MCP
//! wait timeout each commit in their own immediate transaction, and whichever commits first decides
//! the turn. A settlement shares the execution's sequence space with Thread events, so it is written
//! to the outbox in the same transaction that records it.
use super::*;

impl<G: WriteGuard> SessionJournal<G> {
    /// Records `outcome` for one queued `CancelTurn` and, for `not_started`, discards the target
    /// command in the same transaction. Returns the settlement actually recorded: a `not_started`
    /// request for a turn that already failed or was already discarded becomes `already_ended`, so
    /// a cancel never rewrites an earlier result. Repeating an equal settlement returns its original
    /// sequence; a different one for the same command is an identity conflict.
    pub fn settle_turn_cancel(
        &self,
        execution: &ExecutionId,
        command: &CommandId,
        outcome: TurnCancelOutcome,
        target: Option<&CommandId>,
    ) -> Result<(TurnCancelOutcome, Sequence), Error> {
        let mut db = self.lock()?;
        let guard = Arc::clone(&db.guard);
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = read(&tx, execution)?.ok_or(Error::IdentityConflict)?;
        if current.state != ExecutionState::Running {
            return Err(Error::InvalidTransition);
        }
        let input = command_input(&tx, execution, command)?.ok_or(Error::IdentityConflict)?;
        let SessionCommandInput::CancelTurn(cancel) = input else {
            return Err(Error::IdentityConflict);
        };
        let turn = cancel.payload.turn_id;
        if let Some(previous) = settled(&tx, execution, command)? {
            return if previous.0 == outcome || outcome == TurnCancelOutcome::NotStarted {
                Ok(previous)
            } else {
                Err(Error::IdentityConflict)
            };
        }
        // An accepted EndSession wins over everything queued before it (D4); only the ending
        // settlement may be recorded once one exists.
        if outcome != TurnCancelOutcome::SessionEnding && end_requested(&tx, execution)? {
            return Err(Error::InvalidTransition);
        }
        let mut outcome = outcome;
        // `unknown` is only for turns the session never accepted: an accepted turn that left the
        // queue, such as one whose MCP wait timed out, already has its own durable result.
        if outcome == TurnCancelOutcome::Unknown && accepted_turn(&tx, &current, &turn)? {
            outcome = TurnCancelOutcome::AlreadyEnded;
        }
        if outcome == TurnCancelOutcome::NotStarted {
            let failed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM session_turn_failures WHERE execution=?1 AND turn_id=?2)",
                params![execution.as_str(), turn.as_str()],
                |row| row.get(/*idx*/ 0),
            )?;
            let discarded = discarded_turn(&tx, execution, &turn)?;
            match target {
                _ if failed || discarded => outcome = TurnCancelOutcome::AlreadyEnded,
                Some(target) => {
                    let changed = tx.execute("UPDATE session_commands SET state='discarded' WHERE execution=?1 AND command_id=?2 AND state='queued'", params![execution.as_str(), target.as_str()])?;
                    if changed != 1 {
                        // Admission committed first: the caller must treat the turn as started.
                        return Err(Error::InvalidTransition);
                    }
                }
                None if turn != current.command.payload.spec.initial_turn.turn_id => {
                    return Err(Error::IdentityConflict);
                }
                None => {}
            }
        }
        guard.before_write(WritePoint::Progress)?;
        let sequence = write_settlement(
            &tx,
            &guard,
            &current,
            current.last_sequence,
            TurnCancelSettled {
                command_id: command.clone(),
                turn_id: turn,
                outcome,
            },
        )?;
        tx.commit()?;
        Ok((outcome, sequence))
    }

    /// The settlement already recorded for a command, if any.
    pub fn turn_cancel_settlement(
        &self,
        execution: &ExecutionId,
        command: &CommandId,
    ) -> Result<Option<TurnCancelOutcome>, Error> {
        let db = self.lock()?;
        Ok(settled(&db.connection, execution, command)?.map(|(outcome, _)| outcome))
    }
}

impl<G: WriteGuard> SessionJournal<G> {
    /// Whether any session that chose a keep behaviour is still unfinished or still has events no
    /// Controller acknowledged. While one exists, a connection that did not negotiate
    /// `turn_cancel` must not take the sessions over (D5).
    pub fn has_undrained_keep_sessions(&self) -> Result<bool, Error> {
        Ok(self.undrained_keep_sessions()?.iter().any(|session| {
            let spec = &session.command.payload.spec;
            spec.keep_session_on_cancel || spec.keep_session_on_failure
        }))
    }

    /// Whether any session that chose `keep_session_on_failure` is still unfinished or still has
    /// events no Controller acknowledged. While one exists, a connection that did not negotiate
    /// `turn_failure` must not take the sessions over (D6).
    pub fn has_undrained_failure_sessions(&self) -> Result<bool, Error> {
        Ok(self
            .undrained_keep_sessions()?
            .iter()
            .any(|session| session.command.payload.spec.keep_session_on_failure))
    }

    /// All still-unfinished agent sessions, or agent sessions with unacknowledged events.
    fn undrained_keep_sessions(&self) -> Result<Vec<SessionExecution>, Error> {
        let db = self.lock()?;
        db.connection.prepare("SELECT input,state,result,last_sequence FROM node_executions n WHERE kind='agent_session' AND (state<>'completed' OR EXISTS(SELECT 1 FROM execution_events e WHERE e.execution=n.execution)) ORDER BY rowid")?
            .query_map([], row)?
            .map(|r| decode(r?))
            .collect()
    }
}

/// Settles every still-queued `CancelTurn` as `session_ending` in acceptance order, inside the
/// caller's transaction, and returns the execution's new last sequence. Called wherever the
/// session's end is decided, so no `session_ending` can be visible without the decision.
pub(crate) fn settle_pending_as_ending<G: WriteGuard>(
    tx: &rusqlite::Transaction<'_>,
    guard: &Arc<G>,
    current: &SessionExecution,
) -> Result<u64, Error> {
    let pending = tx
        .prepare("SELECT command_id,input FROM session_commands WHERE execution=?1 AND state='queued' ORDER BY acceptance_order")?
        .query_map([current.command.execution_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut last = current.last_sequence;
    for (command, input) in pending {
        let SessionCommandInput::CancelTurn(cancel) = serde_json::from_str(&input)? else {
            continue;
        };
        guard.before_write(WritePoint::Progress)?;
        last = write_settlement(
            tx,
            guard,
            current,
            last,
            TurnCancelSettled {
                command_id: CommandId::new(command),
                turn_id: cancel.payload.turn_id,
                outcome: TurnCancelOutcome::SessionEnding,
            },
        )?
        .value();
    }
    Ok(last)
}

/// Whether the session's end was already decided: the stop end, written together with the stop
/// clock and the cancel settlements, exists.
pub(crate) fn end_decided(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
) -> Result<bool, Error> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_stop_ends WHERE execution=?1)",
        [execution.as_str()],
        |row| row.get(/*idx*/ 0),
    )?)
}

/// Writes one settlement after `last`, marks its command executed and queues the event.
pub(crate) fn write_settlement<G: WriteGuard>(
    tx: &rusqlite::Transaction<'_>,
    guard: &Arc<G>,
    current: &SessionExecution,
    last: u64,
    payload: TurnCancelSettled,
) -> Result<Sequence, Error> {
    let execution = &current.command.execution_id;
    let sequence = last
        .checked_add(1)
        .filter(|s| *s <= i64::MAX as u64)
        .map(Sequence::new)
        .ok_or(Error::InvalidTransition)?;
    let event = NodeToControllerMessage::TurnCancelSettled(TurnCancelSettledMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: current.command.operation_id.clone(),
        execution_id: execution.clone(),
        sequence,
        payload,
    });
    event.validate()?;
    let NodeToControllerMessage::TurnCancelSettled(message) = &event else {
        unreachable!("constructed above");
    };
    let changed = tx.execute(
        "UPDATE session_commands SET state='executed' WHERE execution=?1 AND command_id=?2 AND state='queued'",
        params![execution.as_str(), message.payload.command_id.as_str()],
    )?;
    if changed != 1 {
        return Err(Error::InvalidTransition);
    }
    tx.execute(
        "INSERT INTO session_turn_cancels VALUES(?1,?2,?3,?4,?5)",
        params![
            execution.as_str(),
            message.payload.command_id.as_str(),
            message.payload.turn_id.as_str(),
            outcome_token(message.payload.outcome),
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

/// Reads one stored command input.
fn command_input(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
    command: &CommandId,
) -> Result<Option<SessionCommandInput>, Error> {
    let input: Option<String> = connection
        .query_row(
            "SELECT input FROM session_commands WHERE execution=?1 AND command_id=?2",
            params![execution.as_str(), command.as_str()],
            |row| row.get(/*idx*/ 0),
        )
        .optional()?;
    input
        .map(|input| serde_json::from_str(&input).map_err(Error::from))
        .transpose()
}

/// The recorded settlement and its sequence.
fn settled(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
    command: &CommandId,
) -> Result<Option<(TurnCancelOutcome, Sequence)>, Error> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT outcome,sequence FROM session_turn_cancels WHERE execution=?1 AND command_id=?2",
            params![execution.as_str(), command.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(outcome, sequence)| Ok((parse_outcome(&outcome)?, Sequence::new(sequence as u64))))
        .transpose()
}

/// Whether an `EndSession` was ever accepted for the session.
pub(crate) fn end_requested(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
) -> Result<bool, Error> {
    let inputs = connection
        .prepare("SELECT input FROM session_commands WHERE execution=?1")?
        .query_map([execution.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for input in inputs {
        if matches!(
            serde_json::from_str::<SessionCommandInput>(&input)?,
            SessionCommandInput::EndSession(_)
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the session accepted this turn: its initial turn or any stored user turn command.
fn accepted_turn(
    connection: &rusqlite::Connection,
    current: &SessionExecution,
    turn: &TurnId,
) -> Result<bool, Error> {
    if &current.command.payload.spec.initial_turn.turn_id == turn {
        return Ok(true);
    }
    let inputs = connection
        .prepare("SELECT input FROM session_commands WHERE execution=?1")?
        .query_map([current.command.execution_id.as_str()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for input in inputs {
        if let SessionCommandInput::SubmitUserTurn(message) = serde_json::from_str(&input)?
            && &message.payload.turn.turn_id == turn
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether an earlier cancel already dropped the turn before admission.
pub(super) fn discarded_turn(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
    turn: &TurnId,
) -> Result<bool, Error> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_turn_cancels WHERE execution=?1 AND turn_id=?2 AND outcome='not_started')",
        params![execution.as_str(), turn.as_str()],
        |row| row.get(/*idx*/ 0),
    )?)
}

/// Stable storage token of an outcome; equal to its wire name.
fn outcome_token(outcome: TurnCancelOutcome) -> &'static str {
    match outcome {
        TurnCancelOutcome::Stopped => "stopped",
        TurnCancelOutcome::NotStarted => "not_started",
        TurnCancelOutcome::AlreadyEnded => "already_ended",
        TurnCancelOutcome::Unknown => "unknown",
        TurnCancelOutcome::SessionEnding => "session_ending",
    }
}

/// Inverse of [`outcome_token`]; anything else means the file was not written by this program.
fn parse_outcome(token: &str) -> Result<TurnCancelOutcome, Error> {
    match token {
        "stopped" => Ok(TurnCancelOutcome::Stopped),
        "not_started" => Ok(TurnCancelOutcome::NotStarted),
        "already_ended" => Ok(TurnCancelOutcome::AlreadyEnded),
        "unknown" => Ok(TurnCancelOutcome::Unknown),
        "session_ending" => Ok(TurnCancelOutcome::SessionEnding),
        _ => Err(Error::InvalidSchema),
    }
}

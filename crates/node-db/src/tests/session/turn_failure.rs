//! The one durable settlement of a kept-on-failure session's failed turn.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md cases 9, 11, 12 and 15.
use super::*;
use pretty_assertions::assert_eq;

/// A session that chose to stay after a failed turn, started on this Node.
fn failing() -> (
    tempfile::TempDir,
    NodeDatabase,
    SessionJournal,
    StartAgentSessionMessage,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let mut start = command();
    start.payload.spec.keep_session_on_cancel = true;
    start.payload.spec.keep_session_on_failure = true;
    db.accept_session(&start).unwrap();
    db.start_session(&start, &NodeIncarnationId::new("first"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    (dir, db, journal, start)
}

/// A queued user turn with its own identity.
fn user_turn(command: &str, turn: &str) -> SessionCommandInput {
    let SessionCommandInput::SubmitUserTurn(mut message) = super::turn(command) else {
        unreachable!("a user turn");
    };
    message.payload.turn.turn_id = TurnId::new(turn);
    SessionCommandInput::SubmitUserTurn(message)
}

/// A cancel of one turn.
fn cancel(command: &str, turn: &str) -> SessionCommandInput {
    let start = super::command();
    SessionCommandInput::CancelTurn(CancelTurnMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start.operation_id,
        execution_id: start.execution_id,
        payload: CancelTurn {
            node_id: NodeId::new("node"),
            command_id: CommandId::new(command),
            turn_id: TurnId::new(turn),
        },
    })
}

/// A `TurnEnded` Thread event carrying a failure, as the runtime persisted it.
fn failed_turn_end(turn: &str) -> ThreadEvent {
    ThreadEvent {
        turn_id: Some(TurnId::new(turn)),
        record: serde_json::json!({
            "type": "turnEnded",
            "stop_reason": "cancelled",
            "failure": {
                "code": "agent_error",
                "acp_code": -32001,
                "message": "boom"
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        truncated: false,
        visible_text: None,
        visible_text_truncated: false,
        history_sequence: None,
    }
}

/// Every failure settlement in the outbox, in sequence order.
fn settlements<G: WriteGuard>(db: &NodeDatabase<G>) -> Vec<(u64, TurnFailureSettled)> {
    let mut statement = db
        .connection
        .prepare("SELECT sequence,event FROM execution_events ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .filter_map(|row| {
            let (sequence, event) = row.unwrap();
            match serde_json::from_str::<NodeToControllerMessage>(&event).unwrap() {
                NodeToControllerMessage::TurnFailureSettled(message) => {
                    Some((sequence as u64, message.payload))
                }
                _ => None,
            }
        })
        .collect()
}

/// The conclusions and the terminal in the outbox, in sequence order.
#[derive(Debug, PartialEq)]
enum Concluded {
    Failure(u64, TurnFailureSettled),
    Cancel(u64, TurnCancelSettled),
    Terminal(u64),
}

/// Every failure settlement, cancel settlement and terminal still in the outbox.
fn conclusions<G: WriteGuard>(db: &NodeDatabase<G>) -> Vec<Concluded> {
    let mut statement = db
        .connection
        .prepare("SELECT sequence,event FROM execution_events ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .filter_map(|row| {
            let (sequence, event) = row.unwrap();
            let sequence = sequence as u64;
            match serde_json::from_str::<NodeToControllerMessage>(&event).unwrap() {
                NodeToControllerMessage::TurnFailureSettled(message) => {
                    Some(Concluded::Failure(sequence, message.payload))
                }
                NodeToControllerMessage::TurnCancelSettled(message) => {
                    Some(Concluded::Cancel(sequence, message.payload))
                }
                NodeToControllerMessage::AgentSessionEnded(_) => {
                    Some(Concluded::Terminal(sequence))
                }
                _ => None,
            }
        })
        .collect()
}

/// The `session_ending` conclusion of one failed turn.
fn failure_ending(turn: &str) -> TurnFailureSettled {
    TurnFailureSettled {
        turn_id: TurnId::new(turn),
        outcome: TurnFailureOutcome::SessionEnding,
    }
}

/// The `session_ending` settlement of one merged cancel.
fn cancel_ending(command: &str, turn: &str) -> TurnCancelSettled {
    TurnCancelSettled {
        command_id: CommandId::new(command),
        turn_id: TurnId::new(turn),
        outcome: TurnCancelOutcome::SessionEnding,
    }
}

/// Case 11: a kept failure writes exactly one `kept` before its merged cancels, in one sequence.
#[test]
fn a_kept_failure_settles_beside_its_merged_cancels() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    db.accept_session_command(&user_turn("command-2", "turn-2"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-1", "turn-1"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-2", "turn-1"))
        .unwrap();
    let sequence = journal
        .settle_turn_failure(
            &execution,
            &TurnId::new("turn-1"),
            TurnFailureOutcome::Kept,
            &[CommandId::new("cancel-1"), CommandId::new("cancel-2")],
        )
        .unwrap();
    assert_eq!(
        settlements(&db),
        vec![(
            sequence.value(),
            TurnFailureSettled {
                turn_id: TurnId::new("turn-1"),
                outcome: TurnFailureOutcome::Kept,
            }
        )]
    );
    // The merged cancels settle as `already_ended`; the failure settlement is the transaction head.
    let cancelled = super::turn_cancel::settlements(&db);
    assert_eq!(cancelled.len(), 2);
    assert_eq!(cancelled[0].1.turn_id, TurnId::new("turn-1"));
    assert_eq!(cancelled[0].1.outcome, TurnCancelOutcome::AlreadyEnded);
    assert_eq!(cancelled[1].1.outcome, TurnCancelOutcome::AlreadyEnded);
    assert!(cancelled[0].0 > sequence.value());
    assert!(cancelled[1].0 > cancelled[0].0);
    // A repeated settlement is idempotent.
    assert_eq!(
        journal
            .settle_turn_failure(
                &execution,
                &TurnId::new("turn-1"),
                TurnFailureOutcome::Kept,
                &[],
            )
            .unwrap(),
        sequence
    );
}

/// Case 9: an accepted `EndSession` refuses a `kept` conclusion queued after it.
#[test]
fn an_end_session_refuses_a_kept_settlement() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-1", "turn-1"))
        .unwrap();
    db.accept_session_command(&SessionCommandInput::EndSession(EndSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start.operation_id.clone(),
        execution_id: execution.clone(),
        payload: EndSession {
            node_id: NodeId::new("node"),
            command_id: CommandId::new("end"),
            reason: EndSessionReason::UserEnded,
        },
    }))
    .unwrap();
    assert!(matches!(
        journal.settle_turn_failure(
            &execution,
            &TurnId::new("turn-1"),
            TurnFailureOutcome::Kept,
            &[CommandId::new("cancel-1")],
        ),
        Err(Error::InvalidTransition)
    ));
    assert!(settlements(&db).is_empty());
    assert!(super::turn_cancel::settlements(&db).is_empty());
}

/// Cases 12 and 15: ending a keep-on-failure session whose last Thread event is a failed
/// `turnEnded` without a settlement back-fills exactly one `session_ending` with the terminal.
#[test]
fn a_failed_turn_without_a_settlement_backfills_session_ending() {
    let (_dir, db, journal, start) = failing();
    let execution = start.execution_id.clone();
    journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        settlements(&db),
        vec![(
            terminal.value() - 1,
            TurnFailureSettled {
                turn_id: TurnId::new("turn-1"),
                outcome: TurnFailureOutcome::SessionEnding,
            }
        )]
    );
}

/// Case 9: a turn that already has a `kept` conclusion is not back-filled again at the terminal.
#[test]
fn a_turn_with_a_kept_settlement_is_not_backfilled() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    db.accept_session_command(&cancel("cancel-1", "turn-1"))
        .unwrap();
    journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    let sequence = journal
        .settle_turn_failure(
            &execution,
            &TurnId::new("turn-1"),
            TurnFailureOutcome::Kept,
            &[CommandId::new("cancel-1")],
        )
        .unwrap();
    journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        settlements(&db),
        vec![(
            sequence.value(),
            TurnFailureSettled {
                turn_id: TurnId::new("turn-1"),
                outcome: TurnFailureOutcome::Kept,
            }
        )]
    );
}

/// Case 11 (F5): a turn whose failed `turnEnded` was never persisted cannot be kept, because the
/// Controller never saw the failure the conclusion would answer.
#[test]
fn a_kept_settlement_requires_a_persisted_failure() {
    let (_dir, db, journal, start) = failing();
    let execution = start.execution_id.clone();
    assert!(matches!(
        journal.settle_turn_failure(
            &execution,
            &TurnId::new("turn-1"),
            TurnFailureOutcome::Kept,
            &[],
        ),
        Err(Error::InvalidTransition)
    ));
    assert_eq!(settlements(&db), Vec::new());
    assert_eq!(
        journal
            .turn_failure_settlement(&execution, &TurnId::new("turn-1"))
            .unwrap(),
        None
    );
}

/// Case 15: the Controller acknowledging the failed `turnEnded` while the Node still judges the
/// turn does not remove the pending conclusion; the terminal back-fills it.
#[test]
fn backfill_after_the_failed_turn_end_was_acked() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    let failed = journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    db.acknowledge_session(&ack(failed.value())).unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        conclusions(&db),
        vec![
            Concluded::Failure(failed.value() + 1, failure_ending("turn-1")),
            Concluded::Terminal(terminal.value()),
        ]
    );
}

/// Case 15: after a restart, the `interrupted` end decision back-fills the conclusion of a failure
/// the Controller acknowledged before the restart, exactly once.
#[test]
fn a_restart_backfills_an_acknowledged_failure_once() {
    let (dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    let failed = journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    db.acknowledge_session(&ack(failed.value())).unwrap();
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let journal = db.session_journal().unwrap();
    journal
        .decide_session_end(&execution, &ended(), 1_000, 2_000)
        .unwrap();
    journal
        .decide_session_end(&execution, &ended(), 1_000, 2_000)
        .unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        conclusions(&db),
        vec![
            Concluded::Failure(failed.value() + 1, failure_ending("turn-1")),
            Concluded::Terminal(terminal.value()),
        ]
    );
}

/// Case 9 (fifth scenario): an end decision settles a failed turn before the cancels merged into
/// its stop, as the `kept` transaction does.
#[test]
fn an_end_decision_orders_the_failure_before_merged_cancels() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    let failed = journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-1", "turn-1"))
        .unwrap();
    journal
        .decide_session_end(&execution, &ended(), 1_000, 2_000)
        .unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        conclusions(&db),
        vec![
            Concluded::Failure(failed.value() + 1, failure_ending("turn-1")),
            Concluded::Cancel(failed.value() + 2, cancel_ending("cancel-1", "turn-1")),
            Concluded::Terminal(terminal.value()),
        ]
    );
}

/// Case 12: a terminal written without a deferred decision also settles the failure before the
/// merged cancel.
#[test]
fn backfill_orders_failure_before_merged_cancel() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
    let failed = journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-1", "turn-1"))
        .unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        conclusions(&db),
        vec![
            Concluded::Failure(failed.value() + 1, failure_ending("turn-1")),
            Concluded::Cancel(failed.value() + 2, cancel_ending("cancel-1", "turn-1")),
            Concluded::Terminal(terminal.value()),
        ]
    );
}

/// Case 14: a session that did not choose failure keeping gets the failed `turnEnded` but no
/// conclusion, whatever ends it.
#[test]
fn a_session_without_failure_keeping_gets_no_conclusion() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let mut start = command();
    start.payload.spec.keep_session_on_cancel = true;
    db.accept_session(&start).unwrap();
    db.start_session(&start, &NodeIncarnationId::new("first"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    let execution = start.execution_id.clone();
    journal
        .append_thread_event(&execution, failed_turn_end("turn-1"))
        .unwrap();
    journal
        .decide_session_end(&execution, &ended(), 1_000, 2_000)
        .unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        conclusions(&db),
        vec![Concluded::Terminal(terminal.value())]
    );
}

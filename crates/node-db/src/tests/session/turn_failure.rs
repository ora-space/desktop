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

/// Case 11: a kept failure writes exactly one `kept` before its merged cancels, in one sequence.
#[test]
fn a_kept_failure_settles_beside_its_merged_cancels() {
    let (_dir, mut db, journal, start) = failing();
    let execution = start.execution_id.clone();
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
            &[
                CommandId::new("cancel-1"),
                CommandId::new("cancel-2"),
            ],
        )
        .unwrap();
    assert_eq!(
        settlements(&db),
        vec![(sequence.value(), TurnFailureSettled {
            turn_id: TurnId::new("turn-1"),
            outcome: TurnFailureOutcome::Kept,
        })]
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
    db.accept_session_command(&cancel("cancel-1", "turn-1"))
        .unwrap();
    db.accept_session_command(&SessionCommandInput::EndSession(
        EndSessionMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: start.operation_id.clone(),
            execution_id: execution.clone(),
            payload: EndSession {
                node_id: NodeId::new("node"),
                command_id: CommandId::new("end"),
                reason: EndSessionReason::UserEnded,
            },
        },
    ))
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
        vec![(terminal.value() - 1, TurnFailureSettled {
            turn_id: TurnId::new("turn-1"),
            outcome: TurnFailureOutcome::SessionEnding,
        })]
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
        vec![(sequence.value(), TurnFailureSettled {
            turn_id: TurnId::new("turn-1"),
            outcome: TurnFailureOutcome::Kept,
        })]
    );
}
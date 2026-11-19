//! Durable acceptance and the single settlement of `CancelTurn`.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md; each test names
//! the case it covers.
use super::*;
use pretty_assertions::assert_eq;

/// A session whose input chose to stay after cancelled turns, started on this Node.
fn keeping() -> (
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

/// Every settlement event in the outbox, in sequence order.
fn settlements(db: &NodeDatabase) -> Vec<(u64, TurnCancelSettled)> {
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
                NodeToControllerMessage::TurnCancelSettled(message) => {
                    Some((sequence as u64, message.payload))
                }
                _ => None,
            }
        })
        .collect()
}

/// The settlement a test expects.
fn settled(command: &str, turn: &str, outcome: TurnCancelOutcome) -> TurnCancelSettled {
    TurnCancelSettled {
        command_id: CommandId::new(command),
        turn_id: TurnId::new(turn),
        outcome,
    }
}

/// Cases 20 and 30: a session that did not choose to stay after cancels refuses `CancelTurn`
/// and stores nothing.
#[test]
fn a_session_without_the_cancel_choice_refuses_cancel_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let mut start = command();
    start.payload.spec.keep_session_on_failure = true;
    db.accept_session(&start).unwrap();
    assert_eq!(
        db.accept_session_command(&cancel("cancel", "initial"))
            .unwrap(),
        CommandAdmission::TurnCancelUnsupported
    );
    let journal = db.session_journal().unwrap();
    assert_eq!(
        journal.queued_commands(&start.execution_id).unwrap(),
        vec![]
    );
}

/// Case 2: dropping a queued turn discards its command and writes `not_started` in one
/// transaction; repeating the settlement returns the original sequence.
#[test]
fn a_dropped_turn_is_discarded_with_its_settlement() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    db.accept_session_command(&user_turn("command-2", "turn-2"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-2", "turn-2"))
        .unwrap();
    let first = journal
        .settle_turn_cancel(
            &execution,
            &CommandId::new("cancel-2"),
            TurnCancelOutcome::NotStarted,
            Some(&CommandId::new("command-2")),
        )
        .unwrap();
    let again = journal
        .settle_turn_cancel(
            &execution,
            &CommandId::new("cancel-2"),
            TurnCancelOutcome::NotStarted,
            Some(&CommandId::new("command-2")),
        )
        .unwrap();
    assert_eq!(
        (
            first,
            again,
            journal
                .command_state(&execution, &CommandId::new("command-2"))
                .unwrap(),
            journal.queued_commands(&execution).unwrap(),
            settlements(&db),
        ),
        (
            (TurnCancelOutcome::NotStarted, Sequence::new(1)),
            (TurnCancelOutcome::NotStarted, Sequence::new(1)),
            Some(SessionCommandState::Discarded),
            vec![],
            vec![(
                1,
                settled("cancel-2", "turn-2", TurnCancelOutcome::NotStarted)
            )],
        )
    );
}

/// Case 23: a timeout that committed first wins; the cancel then finds the turn ended and does
/// not rewrite the failure. An admission that committed first refuses the drop.
#[test]
fn the_first_commit_decides_a_waiting_turn() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    db.accept_session_command(&user_turn("command-2", "turn-2"))
        .unwrap();
    db.accept_session_command(&user_turn("command-3", "turn-3"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-2", "turn-2"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-3", "turn-3"))
        .unwrap();
    journal
        .fail_session_turn(
            &execution,
            SessionTurnFailed {
                turn_id: TurnId::new("turn-2"),
                command_id: Some(CommandId::new("command-2")),
                code: SessionTurnFailureCode::McpRefreshWaitTimeout,
            },
        )
        .unwrap();
    let after_timeout = journal
        .settle_turn_cancel(
            &execution,
            &CommandId::new("cancel-2"),
            TurnCancelOutcome::NotStarted,
            Some(&CommandId::new("command-2")),
        )
        .unwrap()
        .0;
    journal
        .settle_command(
            &execution,
            &CommandId::new("command-3"),
            SessionCommandSettlement::Executed,
        )
        .unwrap();
    let after_admission = journal.settle_turn_cancel(
        &execution,
        &CommandId::new("cancel-3"),
        TurnCancelOutcome::NotStarted,
        Some(&CommandId::new("command-3")),
    );
    assert_eq!(after_timeout, TurnCancelOutcome::AlreadyEnded);
    assert!(matches!(after_admission, Err(Error::InvalidTransition)));
    assert_eq!(
        journal
            .turn_cancel_settlement(&execution, &CommandId::new("cancel-3"))
            .unwrap(),
        None
    );
}

/// Cases 19 and 21: the end decision settles every pending cancel as `session_ending` in its
/// own transaction; a cancel accepted after the decision is settled while it is accepted; and
/// an accepted `EndSession` refuses any other settlement.
#[test]
fn the_end_decision_settles_pending_cancels() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    db.accept_session_command(&cancel("cancel-1", "initial"))
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
        journal.settle_turn_cancel(
            &execution,
            &CommandId::new("cancel-1"),
            TurnCancelOutcome::Stopped,
            None
        ),
        Err(Error::InvalidTransition)
    ));
    let mut user_ended = ended();
    user_ended.reason = AgentSessionEndReason::UserEnded;
    journal
        .defer_session_stop_end(&execution, &user_ended)
        .unwrap();
    db.accept_session_command(&cancel("cancel-2", "initial"))
        .unwrap();
    journal
        .defer_session_stop_end(&execution, &user_ended)
        .unwrap();
    assert_eq!(
        settlements(&db),
        vec![
            (
                1,
                settled("cancel-1", "initial", TurnCancelOutcome::SessionEnding)
            ),
            (
                2,
                settled("cancel-2", "initial", TurnCancelOutcome::SessionEnding)
            ),
        ]
    );
}

/// Case 27: a terminal written without a deferred decision, as restart settlement does, settles
/// the pending cancel first, so the settlement precedes the terminal event.
#[test]
fn a_terminal_settles_pending_cancels_before_it() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    journal.append_thread_event(&execution, event()).unwrap();
    db.accept_session_command(&cancel("cancel-1", "initial"))
        .unwrap();
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert_eq!(
        (terminal, settlements(&db)),
        (
            Sequence::new(3),
            vec![(
                2,
                settled("cancel-1", "initial", TurnCancelOutcome::SessionEnding)
            )]
        )
    );
}

/// Case 16: a keep session holds the takeover boundary until it is terminal and every event
/// is acknowledged.
#[test]
fn keep_sessions_hold_the_takeover_boundary_until_drained() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    assert!(journal.has_undrained_keep_sessions().unwrap());
    let terminal = journal.end_session(&execution, ended()).unwrap();
    assert!(journal.has_undrained_keep_sessions().unwrap());
    db.acknowledge_session(&ack(terminal.value())).unwrap();
    assert!(!journal.has_undrained_keep_sessions().unwrap());
}

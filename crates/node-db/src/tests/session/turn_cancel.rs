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
fn settlements<G: WriteGuard>(db: &NodeDatabase<G>) -> Vec<(u64, TurnCancelSettled)> {
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
        .decide_session_end(&execution, &user_ended, 1_000, 2_000)
        .unwrap();
    db.accept_session_command(&cancel("cancel-2", "initial"))
        .unwrap();
    journal
        .decide_session_end(&execution, &user_ended, 1_000, 2_000)
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

/// The end decision of an unanswered cancel.
fn unanswered() -> AgentSessionEnded {
    let mut ended = ended();
    ended.reason = AgentSessionEndReason::AgentFailed;
    ended.detail = Some("turn_cancel_unanswered".into());
    ended
}

/// Case 28: after the decision and its `session_ending` are durable, a restart keeps the
/// original decision, adds no second settlement, and the terminal carries the original cause.
#[test]
fn a_restart_after_the_settlement_keeps_the_original_decision() {
    let (dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    db.accept_session_command(&cancel("cancel-1", "initial"))
        .unwrap();
    journal
        .decide_session_end(&execution, &unanswered(), 1_000, 2_000)
        .unwrap();
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let journal = db.session_journal().unwrap();
    let decision = journal.session_stop_end(&execution).unwrap().unwrap();
    // Recovery may record its own intention again; the first decision stays.
    journal
        .decide_session_end(&execution, &ended(), 1_000, 2_000)
        .unwrap();
    assert_eq!(
        journal.session_stop_end(&execution).unwrap(),
        Some(decision.clone())
    );
    journal.end_session(&execution, decision.clone()).unwrap();
    assert_eq!(
        (decision.detail, settlements(&db)),
        (
            Some("turn_cancel_unanswered".to_string()),
            vec![(
                1,
                settled("cancel-1", "initial", TurnCancelOutcome::SessionEnding)
            )]
        )
    );
}

/// Case 29: while the ledger cannot write, nothing is settled from memory; once it can, the end
/// decision writes exactly one `session_ending` per unsettled cancel and leaves an earlier
/// settlement alone.
#[test]
fn an_unwritable_ledger_settles_only_after_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let fault = std::rc::Rc::new(std::cell::Cell::new(/*value*/ None));
    let mut db = NodeDatabase::open_with_guard(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
        crate::tests::Fault(fault.clone()),
    )
    .unwrap();
    let mut start = command();
    start.payload.spec.keep_session_on_cancel = true;
    db.accept_session(&start).unwrap();
    db.start_session(&start, &NodeIncarnationId::new("first"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    let execution = start.execution_id.clone();
    db.accept_session_command(&user_turn("command-2", "turn-2"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-early", "turn-x"))
        .unwrap();
    journal
        .settle_turn_cancel(
            &execution,
            &CommandId::new("cancel-early"),
            TurnCancelOutcome::Unknown,
            None,
        )
        .unwrap();
    db.accept_session_command(&cancel("cancel-1", "turn-2"))
        .unwrap();
    fault.set(Some(WritePoint::Progress));
    assert!(
        journal
            .settle_turn_cancel(
                &execution,
                &CommandId::new("cancel-1"),
                TurnCancelOutcome::NotStarted,
                Some(&CommandId::new("command-2")),
            )
            .is_err()
    );
    let mut ledger_unavailable = ended();
    ledger_unavailable.reason = AgentSessionEndReason::AgentFailed;
    ledger_unavailable.detail = Some("ledger_unavailable".into());
    assert!(
        journal
            .decide_session_end(&execution, &ledger_unavailable, 1_000, 2_000)
            .is_err()
    );
    let unwritten = settlements(&db);
    fault.set(/*val*/ None);
    journal
        .decide_session_end(&execution, &ledger_unavailable, 1_000, 2_000)
        .unwrap();
    journal
        .decide_session_end(&execution, &ledger_unavailable, 1_000, 2_000)
        .unwrap();
    assert_eq!(
        (
            unwritten,
            settlements(&db),
            journal
                .command_state(&execution, &CommandId::new("command-2"))
                .unwrap()
        ),
        (
            vec![(
                1,
                settled("cancel-early", "turn-x", TurnCancelOutcome::Unknown)
            )],
            vec![
                (
                    1,
                    settled("cancel-early", "turn-x", TurnCancelOutcome::Unknown)
                ),
                (
                    2,
                    settled("cancel-1", "turn-2", TurnCancelOutcome::SessionEnding)
                ),
            ],
            Some(SessionCommandState::Queued)
        )
    );
}

/// One Agent stop responsibility of the session.
fn attempt(start: &StartAgentSessionMessage) -> SessionStopIdentity {
    SessionStopIdentity {
        attempt: AttemptId::generate(),
        execution: start.execution_id.clone(),
        checkout: start.payload.spec.checkout_execution_id.clone(),
        node: NodeRuntimeIdentity {
            node_id: start.payload.spec.node_id.clone(),
            incarnation_id: NodeIncarnationId::new("first"),
        },
        host: SessionStopHost {
            boot_id: "boot".into(),
            trusted_host_id: Some("host".into()),
        },
        kind: SessionStopKind::Agent,
        packages: vec![SessionStopPackage {
            plugin_id: start.payload.spec.agent_plugin_id.clone(),
            plugin_version: start.payload.spec.agent_plugin_version.clone(),
            view_key: None,
        }],
    }
}

/// Case 28 (stop clock): the decision, the stop clock of the open stop responsibility and the
/// cancel settlement commit together, so a restart right after `session_ending` became visible
/// finds the original clock, and later stop requests never move it.
#[test]
fn the_stop_clock_commits_with_the_decision_and_survives_restart() {
    let (dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    let identity = attempt(&start);
    journal
        .record_session_stop(&identity, SessionStopScope::Recorded, 500)
        .unwrap();
    db.accept_session_command(&cancel("cancel-1", "initial"))
        .unwrap();
    journal
        .decide_session_end(&execution, &unanswered(), 1_000, 601_000)
        .unwrap();
    let clock = |journal: &SessionJournal| {
        let records = journal.session_stop_records(Some(&execution)).unwrap();
        (records[0].first_stop_at_ms, records[0].bound_at_ms)
    };
    let decided = (clock(&journal), settlements(&db));
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let journal = db.session_journal().unwrap();
    journal
        .change_session_stop(
            &identity.attempt,
            SessionStopChange::Request {
                at_ms: 9_000,
                bound_at_ms: 609_000,
            },
        )
        .unwrap();
    journal
        .decide_session_end(&execution, &ended(), 9_000, 609_000)
        .unwrap();
    assert_eq!(
        decided,
        (
            (Some(1_000), Some(601_000)),
            vec![(
                1,
                settled("cancel-1", "initial", TurnCancelOutcome::SessionEnding)
            )]
        )
    );
    assert_eq!(
        (clock(&journal), settlements(&db).len()),
        ((Some(1_000), Some(601_000)), 1)
    );
}

/// Neither a restore-owned end intention nor a stop end recorded without its clock settles a
/// cancel, so a restart before the full decision recovers the cancel instead of leaving a
/// `session_ending` with no recoverable decision.
#[test]
fn only_the_full_decision_settles_cancels() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    db.accept_session_command(&cancel("cancel-1", "initial"))
        .unwrap();
    let mut failed = ended();
    failed.reason = AgentSessionEndReason::AgentFailed;
    failed.detail = Some("agent_start_failed".into());
    journal.defer_restore_end(&execution, &failed).unwrap();
    journal.defer_session_stop_end(&execution, &failed).unwrap();
    assert_eq!(settlements(&db), vec![]);
    journal
        .decide_session_end(&execution, &failed, 1_000, 2_000)
        .unwrap();
    assert_eq!(
        settlements(&db),
        vec![(
            1,
            settled("cancel-1", "initial", TurnCancelOutcome::SessionEnding)
        )]
    );
}

/// Case 23 (timeout behind the head): a turn whose MCP wait timed out while queued behind
/// another has its own result, so a later cancel of it is `already_ended`, never `unknown`.
#[test]
fn a_cancel_of_a_timed_out_queued_turn_is_already_ended() {
    let (_dir, mut db, journal, start) = keeping();
    let execution = start.execution_id.clone();
    db.accept_session_command(&user_turn("command-2", "turn-2"))
        .unwrap();
    db.accept_session_command(&user_turn("command-3", "turn-3"))
        .unwrap();
    journal
        .fail_session_turn(
            &execution,
            SessionTurnFailed {
                turn_id: TurnId::new("turn-3"),
                command_id: Some(CommandId::new("command-3")),
                code: SessionTurnFailureCode::McpRefreshWaitTimeout,
            },
        )
        .unwrap();
    db.accept_session_command(&cancel("cancel-3", "turn-3"))
        .unwrap();
    db.accept_session_command(&cancel("cancel-x", "turn-x"))
        .unwrap();
    let settle = |command: &str| {
        journal
            .settle_turn_cancel(
                &execution,
                &CommandId::new(command),
                TurnCancelOutcome::Unknown,
                None,
            )
            .unwrap()
            .0
    };
    assert_eq!(
        (settle("cancel-3"), settle("cancel-x")),
        (TurnCancelOutcome::AlreadyEnded, TurnCancelOutcome::Unknown)
    );
}

/// A cancel whose acceptance write fails is not stored and gets no settlement, even when the
/// end is already decided and acceptance would have settled it in the same transaction; the
/// protocol side therefore never replies that it was accepted.
#[test]
fn a_failed_acceptance_stores_and_settles_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fault = std::rc::Rc::new(std::cell::Cell::new(/*value*/ None));
    let mut db = NodeDatabase::open_with_guard(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
        crate::tests::Fault(fault.clone()),
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
        .decide_session_end(&execution, &unanswered(), 1_000, 2_000)
        .unwrap();
    fault.set(Some(WritePoint::Accept));
    assert!(matches!(
        db.accept_session_command(&cancel("cancel-1", "initial")),
        Err(Error::Injected(WritePoint::Accept))
    ));
    fault.set(/*val*/ None);
    assert_eq!(
        (
            journal
                .command_state(&execution, &CommandId::new("cancel-1"))
                .unwrap(),
            settlements(&db)
        ),
        (None, vec![])
    );
}

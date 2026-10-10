//! Real SQLite coverage of session durability, queue settlement and restart boundaries.
use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
mod recovery;

/// Supplies valid immutable input independently of the agent runtime.
pub(super) fn command() -> StartAgentSessionMessage {
    StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("session-op"),
        execution_id: ExecutionId::new("session-execution"),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                node_id: NodeId::new("node"),
                agent_plugin_id: PluginId::new("official/agent"),
                agent_plugin_version: PluginVersion::new("1.0.0"),
                checkout_execution_id: ExecutionId::new("clone"),
                git_identity: GitIdentity {
                    name: "User".into(),
                    email: "user@example.com".into(),
                },
                initial_turn: UserTurn {
                    turn_id: TurnId::new("initial"),
                    content: vec![ContentBlock::Text {
                        text: "hello".into(),
                    }],
                },
            },
        },
    }
}

/// Creates a queue entry whose ID can differ from its acceptance order.
fn turn(id: &str) -> SessionCommandInput {
    let start = command();
    SessionCommandInput::SubmitUserTurn(SubmitUserTurnMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start.operation_id,
        execution_id: start.execution_id,
        payload: SubmitUserTurn {
            node_id: NodeId::new("node"),
            command_id: CommandId::new(id),
            turn: start.payload.spec.initial_turn,
        },
    })
}

/// Supplies a valid event without coupling storage tests to history serialization.
fn event() -> ThreadEvent {
    ThreadEvent {
        turn_id: None,
        record: serde_json::Map::new(),
        truncated: false,
    }
}

/// Interrupted recovery may seal both accepted and running sessions.
pub(super) fn ended() -> AgentSessionEnded {
    AgentSessionEnded {
        node: NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("restart"),
        },
        reason: AgentSessionEndReason::Interrupted,
        detail: None,
    }
}

/// Constructs an exact ACK; changing one identity must never delete another event.
fn ack(sequence: u64) -> EventAckMessage {
    EventAckMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command().operation_id,
        execution_id: command().execution_id,
        sequence: Sequence::new(sequence),
        payload: EventAck {
            node_id: NodeId::new("node"),
        },
    }
}

/// Event deletion never resets sequencing, and the terminal result remains after the last ACK.
#[test]
fn exact_ack_and_sequence_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let owner = ControllerId::new("owner");
    db.bind_controller(&owner).unwrap();
    let input = command();
    let accepted = db.accept_session(&input).unwrap();
    assert_eq!(db.accept_session(&input).unwrap(), accepted);
    let journal = db.session_journal().unwrap();
    assert!(
        journal
            .append_thread_event(&input.execution_id, event())
            .is_err()
    );
    assert!(
        db.start_session(&input, &NodeIncarnationId::new("first"))
            .unwrap()
    );
    for number in 1..=3 {
        assert_eq!(
            journal
                .append_thread_event(&input.execution_id, event())
                .unwrap(),
            Sequence::new(number)
        );
    }
    let events = db.pending_events().unwrap();
    assert_eq!(db.controller_events(&owner).unwrap(), events);
    db.acknowledge(&ack(/*sequence*/ 2)).unwrap();
    db.acknowledge(&ack(/*sequence*/ 2)).unwrap();
    assert_eq!(
        db.pending_events().unwrap(),
        vec![events[0].clone(), events[2].clone()]
    );
    assert!(db.acknowledge(&ack(/*sequence*/ 4)).is_err());
    let mut wrong = ack(/*sequence*/ 1);
    wrong.operation_id = OperationId::new("other");
    assert!(db.acknowledge(&wrong).is_err());
    wrong = ack(/*sequence*/ 1);
    wrong.payload.node_id = NodeId::new("other");
    assert!(db.acknowledge(&wrong).is_err());
    db.acknowledge(&ack(/*sequence*/ 1)).unwrap();
    db.acknowledge(&ack(/*sequence*/ 3)).unwrap();
    drop(journal);
    drop(db);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert!(db.pending_events().unwrap().is_empty());
    let journal = db.session_journal().unwrap();
    assert_eq!(
        journal
            .append_thread_event(&input.execution_id, event())
            .unwrap(),
        Sequence::new(/*value*/ 4)
    );
    assert_eq!(
        journal.end_session(&input.execution_id, ended()).unwrap(),
        Sequence::new(/*value*/ 5)
    );
    assert!(
        journal
            .append_thread_event(&input.execution_id, event())
            .is_err()
    );
    assert!(journal.end_session(&input.execution_id, ended()).is_err());
    db.acknowledge(&ack(/*sequence*/ 5)).unwrap();
    db.acknowledge(&ack(/*sequence*/ 4)).unwrap();
    assert_eq!(
        db.find_session(&input.operation_id, &input.execution_id)
            .unwrap(),
        Some(SessionExecution {
            command: input.clone(),
            state: ExecutionState::Completed(ExecutionResult::AgentSession(
                AgentSessionResult::AgentSessionEnded(ended())
            )),
            last_sequence: 5
        })
    );
    assert!(db.recoverable_sessions().unwrap().is_empty());
    assert!(db.unfinished_runtime_executions().unwrap().is_empty());
}

/// Full admission order survives deduplication, restart and mutually exclusive settlement.
#[test]
fn commands_are_ordered_deduplicated_and_discarded_at_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    db.accept_session(&command()).unwrap();
    let first = turn("z-first");
    let second = turn("a-second");
    let end = SessionCommandInput::EndSession(EndSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command().operation_id,
        execution_id: command().execution_id,
        payload: EndSession {
            node_id: NodeId::new("node"),
            command_id: CommandId::new("end"),
            reason: EndSessionReason::UserEnded,
        },
    });
    for input in [&first, &second, &end, &first] {
        assert_eq!(
            db.accept_session_command(input).unwrap(),
            CommandAdmission::Accepted
        );
    }
    drop(db);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let journal = db.session_journal().unwrap();
    let execution = command().execution_id;
    assert_eq!(
        journal.queued_commands(&execution).unwrap(),
        vec![first.clone(), second.clone(), end.clone()]
    );
    journal
        .settle_command(
            &execution,
            first.command_id(),
            SessionCommandSettlement::Executed,
        )
        .unwrap();
    journal
        .settle_command(
            &execution,
            first.command_id(),
            SessionCommandSettlement::Executed,
        )
        .unwrap();
    assert!(
        journal
            .settle_command(
                &execution,
                first.command_id(),
                SessionCommandSettlement::Discarded
            )
            .is_err()
    );
    assert_eq!(
        db.accept_session_command(&first).unwrap(),
        CommandAdmission::Accepted
    );
    let mut changed = first.clone();
    if let SessionCommandInput::SubmitUserTurn(m) = &mut changed {
        m.payload.turn.turn_id = TurnId::new("changed");
    }
    assert!(db.accept_session_command(&changed).is_err());
    assert_eq!(
        journal.queued_commands(&execution).unwrap(),
        vec![second.clone(), end.clone()]
    );
    journal.end_session(&execution, ended()).unwrap();
    assert!(journal.queued_commands(&execution).unwrap().is_empty());
    assert_eq!(
        journal
            .command_state(&execution, first.command_id())
            .unwrap(),
        Some(SessionCommandState::Executed)
    );
    for input in [&second, &end] {
        assert_eq!(
            journal
                .command_state(&execution, input.command_id())
                .unwrap(),
            Some(SessionCommandState::Discarded)
        );
        assert!(
            journal
                .settle_command(
                    &execution,
                    input.command_id(),
                    SessionCommandSettlement::Executed
                )
                .is_err()
        );
    }
    assert_eq!(
        db.accept_session_command(&turn("late")).unwrap(),
        CommandAdmission::SessionEnded
    );
    assert_eq!(
        journal
            .command_state(&execution, &CommandId::new("late"))
            .unwrap(),
        None
    );
}

/// A failed outbox insertion cannot leave a sequence gap, a terminal state, or discarded commands.
#[test]
fn failed_event_rolls_back_all_session_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let fault = std::rc::Rc::new(std::cell::Cell::new(/*value*/ None));
    let mut db = NodeDatabase::open_with_guard(
        &path,
        NodeIdentity::Require(NodeId::new("node")),
        Fault(fault.clone()),
    )
    .unwrap();
    let input = command();
    db.accept_session(&input).unwrap();
    db.start_session(&input, &NodeIncarnationId::new("first"))
        .unwrap();
    db.accept_session_command(&turn("queued")).unwrap();
    let journal = db.session_journal().unwrap();
    fault.set(Some(WritePoint::Outbox));
    assert!(
        journal
            .append_thread_event(&input.execution_id, event())
            .is_err()
    );
    assert!(journal.end_session(&input.execution_id, ended()).is_err());
    assert_eq!(
        db.find_session(&input.operation_id, &input.execution_id)
            .unwrap(),
        Some(SessionExecution {
            command: input.clone(),
            state: ExecutionState::Running,
            last_sequence: 0
        })
    );
    assert_eq!(
        journal.queued_commands(&input.execution_id).unwrap(),
        vec![turn("queued")]
    );
    assert!(db.pending_events().unwrap().is_empty());
    fault.set(/*val*/ None);
    assert_eq!(
        journal.end_session(&input.execution_id, ended()).unwrap(),
        Sequence::new(/*value*/ 1)
    );
}

/// Separate actor connections serialize sequence allocation and retain the exclusive owner lease.
#[test]
fn concurrent_journals_share_sequences_and_owner_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    db.accept_session(&command()).unwrap();
    db.start_session(&command(), &NodeIncarnationId::new("first"))
        .unwrap();
    let first = db.session_journal().unwrap();
    let second = db.session_journal().unwrap();
    let keep = first.clone();
    drop(db);
    assert!(matches!(
        NodeDatabase::open(&path, NodeIdentity::Discover),
        Err(Error::AlreadyRunning)
    ));
    let threads: Vec<_> = [first, second]
        .into_iter()
        .map(|journal| {
            std::thread::spawn(move || {
                (0..20)
                    .map(|_| {
                        journal
                            .append_thread_event(&command().execution_id, event())
                            .unwrap()
                            .value()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut sequences: Vec<_> = threads
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=40).collect::<Vec<_>>());
    drop(keep);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(db.pending_events().unwrap().len(), 40);
}

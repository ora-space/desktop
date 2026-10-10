#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
//! Exercises the runtime ports through their production SQLite adapter.
use ora_node::{CheckoutResolver, CommandSettlement, QueuedCommand, SessionCommand, SessionLedger};
use ora_node_db::{NodeDatabase, NodeIdentity, SessionCommandInput};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// The runtime-facing trait preserves command input and commits before returning each sequence.
#[test]
fn runtime_ports_use_durable_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let turn = UserTurn {
        turn_id: TurnId::new("turn"),
        content: vec![ContentBlock::Text {
            text: "hello".into(),
        }],
    };
    let input = StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("operation"),
        execution_id: ExecutionId::new("execution"),
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
                initial_turn: turn.clone(),
                prior_revision: None,
            },
        },
    };
    db.accept_session(&input).unwrap();
    db.start_session(&input, &NodeIncarnationId::new("first"))
        .unwrap();
    let command_id = CommandId::new("command");
    db.accept_session_command(&SessionCommandInput::SubmitUserTurn(
        SubmitUserTurnMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: input.operation_id.clone(),
            execution_id: input.execution_id.clone(),
            payload: SubmitUserTurn {
                node_id: NodeId::new("node"),
                command_id: command_id.clone(),
                turn: turn.clone(),
            },
        },
    ))
    .unwrap();
    let ledger = db.session_journal().unwrap();
    assert_eq!(
        SessionLedger::queued_commands(&ledger, &input.execution_id).unwrap(),
        vec![QueuedCommand {
            command_id: command_id.clone(),
            command: SessionCommand::SubmitUserTurn(turn)
        }]
    );
    assert_eq!(
        CheckoutResolver::checkout(&ledger, &input.payload.spec.checkout_execution_id),
        None
    );
    let event = ThreadEvent {
        turn_id: None,
        record: serde_json::Map::new(),
        truncated: false,
    };
    assert_eq!(
        SessionLedger::append_thread_event(&ledger, &input.execution_id, event.clone()).unwrap(),
        Sequence::new(/*value*/ 1)
    );
    SessionLedger::settle_command(
        &ledger,
        &input.execution_id,
        &command_id,
        CommandSettlement::Executed,
    )
    .unwrap();
    assert!(
        SessionLedger::queued_commands(&ledger, &input.execution_id)
            .unwrap()
            .is_empty()
    );
    let ended = AgentSessionEnded {
        node: NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("first"),
        },
        reason: AgentSessionEndReason::UserEnded,
        detail: None,
    };
    assert_eq!(
        SessionLedger::end_session(&ledger, &input.execution_id, ended).unwrap(),
        Sequence::new(/*value*/ 2)
    );
    assert!(SessionLedger::append_thread_event(&ledger, &input.execution_id, event).is_err());
    let expected = db.pending_events().unwrap();
    drop(ledger);
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(db.pending_events().unwrap(), expected);
}

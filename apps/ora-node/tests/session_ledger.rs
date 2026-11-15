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
                mcp: Vec::new(),
                agent_configuration: None,
                node_id: NodeId::new("node"),
                agent_plugin_id: PluginId::new("official/agent"),
                agent_plugin_version: PluginVersion::new("1.0.0"),
                checkout_execution_id: ExecutionId::new("clone"),
                model_binding_id: None,
                git_identity: GitIdentity {
                    name: "User".into(),
                    email: "user@example.com".into(),
                },
                initial_turn: turn.clone(),
                prior_revision: None,
                workspace_directory: vec![],
                trigger: None,
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
        visible_text: None,
        visible_text_truncated: false,
        history_sequence: None,
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
        plugin_id: None,
        setting_id: None,
        transport: None,
        scheduled_result: None,
        artifact_manifest: None,
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

/// The full control frame stays in memory; SQLite, WAL and replay contain identities only.
#[test]
fn session_mcp_settings_never_enter_the_durable_ledger() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db");
        let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
        let mut input: StartAgentSessionMessage = serde_json::from_value(serde_json::json!({
            "protocol_version":1, "operation_id":"operation", "execution_id":"execution", "payload":{"spec":{
                "node_id":"node", "agent_plugin_id":"official/agent", "agent_plugin_version":"1.0.0",
                "checkout_execution_id":"clone", "git_identity":{"name":"User", "email":"user@example.com"},
                "initial_turn":{"turn_id":"turn", "content":[{"type":"text", "text":"hello"}]},
                "mcp":[{"plugin_id":"official/acme.mcp", "plugin_version":"1.0.0", "settings":{"token":"m938-ledger-secret"}}]
            }}
        })).unwrap();
        let mut durable = input.clone();
        durable.payload.spec.mcp[0].settings.clear();
        let record = db.accept_session(&input).unwrap();
        assert_eq!(record.command, durable);
        // A retransmission cannot use a new value to restart or reconfigure this execution.
        input.payload.spec.mcp[0].settings.insert(
            "token".into(),
            SettingValue::String("m938-ledger-other-secret".into()),
        );
        assert_eq!(db.accept_session(&input).unwrap(), record);
        db.start_session(&input, &NodeIncarnationId::new("first"))
            .unwrap();
        let mut conflict = input.clone();
        conflict.payload.spec.mcp[0].plugin_version = PluginVersion::new("2.0.0");
        assert!(db.accept_session(&conflict).is_err());
        for entry in std::fs::read_dir(root.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let bytes = std::fs::read(path).unwrap();
                for secret in [
                    b"m938-ledger-secret".as_slice(),
                    b"m938-ledger-other-secret".as_slice(),
                ] {
                    assert!(!bytes.windows(secret.len()).any(|window| window == secret));
                }
            }
        }
        drop(db);
        let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
        let record = db.recoverable_sessions().unwrap().pop().unwrap();
        assert_eq!(record.command, durable);
    });
}

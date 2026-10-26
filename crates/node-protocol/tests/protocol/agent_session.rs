use super::session::node;
use super::support::*;
use ora_node_protocol::*;
use serde_json::{Map, Value, json};

const OPERATION: &str = "run-1";
const EXECUTION: &str = "execution-session";

/// Builds a single-text user turn.
fn turn(turn_id: &str, text: &str) -> UserTurn {
    UserTurn {
        turn_id: TurnId::new(turn_id),
        content: vec![ContentBlock::Text {
            text: text.to_owned(),
        }],
    }
}

/// A session start with its git identity and first turn, with independent JSON.
pub(super) fn start() -> Case {
    Case {
        message: Message::Controller(ControllerToNodeMessage::StartAgentSession(
            StartAgentSessionMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: StartAgentSession {
                    spec: AgentSessionSpec {
                        agent_configuration: None,
                        node_id: NodeId::new("node-1"),
                        agent_plugin_id: PluginId::new("ora.1a2b/claude-code"),
                        agent_plugin_version: PluginVersion::new("1.2.0"),
                        checkout_execution_id: ExecutionId::new("execution-clone"),
                        model_binding_id: None,
                        git_identity: GitIdentity {
                            name: "Ada Lovelace".to_owned(),
                            email: "ada@example.com".to_owned(),
                        },
                        initial_turn: turn("turn-1", "Fix the failing login test."),
                        prior_revision: None,
                        workspace_directory: Vec::new(),
                        trigger: None,
                    },
                },
            },
        )),
        wire: json!({
            "message_type": "start_agent_session",
            "protocol_version": 1,
            "operation_id": OPERATION,
            "execution_id": EXECUTION,
            "payload": {"spec": {
                "node_id": "node-1",
                "agent_plugin_id": "ora.1a2b/claude-code",
                "agent_plugin_version": "1.2.0",
                "checkout_execution_id": "execution-clone",
                "git_identity": {"name": "Ada Lovelace", "email": "ada@example.com"},
                "initial_turn": {"turn_id": "turn-1",
                    "content": [{"type": "text", "text": "Fix the failing login test."}]}
            }}
        }),
    }
}

/// One Thread event carrying an opaque history record that belongs to a turn.
fn thread_event() -> Case {
    let record = json!({"type": "agent_message", "text": "Looking at the test."});
    let Value::Object(record) = record else {
        panic!("fixture record must be an object")
    };
    Case {
        message: Message::Node(NodeToControllerMessage::ThreadEvent(ThreadEventMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new(OPERATION),
            execution_id: ExecutionId::new(EXECUTION),
            sequence: Sequence::new(/*value*/ 3),
            payload: ThreadEvent {
                turn_id: Some(TurnId::new("turn-1")),
                record,
                truncated: false,
                visible_text: None,
                visible_text_truncated: false,
                history_sequence: None,
            },
        })),
        wire: json!({"message_type": "thread_event", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION, "sequence": 3,
            "payload": {"turn_id": "turn-1",
                "record": {"type": "agent_message", "text": "Looking at the test."}}}),
    }
}

/// The start command has a fixed wire shape and the shared envelope guarantees.
#[tokio::test]
async fn start_agent_session_round_trips() -> Result<(), TestError> {
    let case = start();
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.assert_envelope_rejections().await?;
    case.assert_fields(
        &[
            "/payload/spec/agent_plugin_id",
            "/payload/spec/checkout_execution_id",
            "/payload/spec/git_identity",
            "/payload/spec/initial_turn",
        ],
        &[
            ("/operation_id", "operation_id"),
            ("/execution_id", "execution_id"),
            ("/payload/spec/node_id", "node_id"),
            (
                "/payload/spec/checkout_execution_id",
                "checkout_execution_id",
            ),
            ("/payload/spec/initial_turn/turn_id", "turn_id"),
        ],
    )
    .await
}

/// The Workspace directory crosses as given; only its identities are checked, so `ora-git list`
/// never attaches one Workspace's Issue to another's branches.
#[tokio::test]
async fn workspace_directory_round_trips_and_rejects_ambiguous_identities() -> Result<(), TestError>
{
    let main = "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a";
    let run = "5d4c3b2a-1f0e-4d8c-9b7a-6f5e4d3c2b1a";
    let mut case = start();
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(command)) =
        &mut case.message
    else {
        panic!("fixture must be a session start")
    };
    command.payload.spec.workspace_directory = vec![
        WorkspaceListing {
            workflow_run_id: None,
            workflow_name: None,
            workspace_id: run.to_owned(),
            kind: "run".to_owned(),
            issue_id: Some("issue-1".to_owned()),
            issue_title: Some("Fix the login test".to_owned()),
            issue_run_id: Some("run-1".to_owned()),
            run_status: Some("completed".to_owned()),
        },
        WorkspaceListing {
            workflow_run_id: None,
            workflow_name: None,
            workspace_id: main.to_owned(),
            kind: "main".to_owned(),
            issue_id: None,
            issue_title: None,
            issue_run_id: None,
            run_status: None,
        },
    ];
    case.wire["payload"]["spec"]["workspace_directory"] = json!([
        {"workspace_id": run, "kind": "run", "issue_id": "issue-1",
            "issue_title": "Fix the login test", "issue_run_id": "run-1",
            "run_status": "completed"},
        {"workspace_id": main, "kind": "main"}
    ]);
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    for directory in [
        json!([{"workspace_id": "workspace-1", "kind": "main"}]),
        json!([{"workspace_id": main.to_uppercase(), "kind": "main"}]),
        json!([{"workspace_id": main, "kind": "main"}, {"workspace_id": main, "kind": "run"}]),
        json!((0..101)
            .map(|index| json!({"workspace_id": format!("{index:08x}-0000-4000-8000-000000000000"), "kind": "run"}))
            .collect::<Vec<_>>()),
    ] {
        let mut wire = case.wire.clone();
        wire["payload"]["spec"]["workspace_directory"] = directory;
        reject_semantics(
            Peer::Controller,
            &wire,
            "workspace directory",
            MessageValidationError::InvalidWorkspaceDirectory,
        )
        .await?;
    }
    Ok(())
}

/// An optional model binding crosses the control wire as an opaque reference, never credentials.
#[tokio::test]
async fn model_bound_session_round_trips_and_rejects_empty_binding() -> Result<(), TestError> {
    let mut case = start();
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(command)) =
        &mut case.message
    else {
        panic!("fixture must be a session start")
    };
    command.payload.spec.model_binding_id = Some(ModelBindingId::new("binding-1"));
    case.wire["payload"]["spec"]["model_binding_id"] = json!("binding-1");
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.wire["payload"]["spec"]["model_binding_id"] = json!("");
    reject_semantics(
        Peer::Controller,
        &case.wire,
        "/payload/spec/model_binding_id",
        MessageValidationError::EmptyField {
            field: "model_binding_id",
        },
    )
    .await
}

/// A frozen adapter configuration crosses with its binding, Settings kept as exact JSON text.
#[tokio::test]
async fn configured_session_round_trips_and_rejects_inconsistent_configuration()
-> Result<(), TestError> {
    let mut case = start();
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(command)) =
        &mut case.message
    else {
        panic!("fixture must be a session start")
    };
    command.payload.spec.model_binding_id = Some(ModelBindingId::new("binding-1"));
    command.payload.spec.agent_configuration = Some(FrozenAgentConfiguration {
        plugin_id: PluginId::new("ora.1a2b/claude-code"),
        plugin_version: PluginVersion::new("1.2.0"),
        target: "x86_64-unknown-linux-gnu".into(),
        package_sha256: Sha256Digest::new("a".repeat(64)),
        schema_version: 2,
        declaration_fingerprint: Sha256Digest::new("b".repeat(64)),
        adapter_version: MODEL_ADAPTER_CONTRACT_VERSION,
        configuration_revision: 3,
        settings_json: r#"{"limit":12345678901234567890}"#.into(),
        protocol: ModelProtocol::AnthropicMessages,
        model: BoundModel {
            id: "team/model".into(),
            name: "Team model".into(),
            context_window: 200_000,
            max_tokens: 8192,
        },
    });
    case.wire["payload"]["spec"]["model_binding_id"] = json!("binding-1");
    case.wire["payload"]["spec"]["agent_configuration"] = json!({
        "plugin_id": "ora.1a2b/claude-code",
        "plugin_version": "1.2.0",
        "target": "x86_64-unknown-linux-gnu",
        "package_sha256": "a".repeat(64),
        "schema_version": 2,
        "declaration_fingerprint": "b".repeat(64),
        "adapter_version": 1,
        "configuration_revision": 3,
        "settings_json": r#"{"limit":12345678901234567890}"#,
        "protocol": "anthropic-messages",
        "model": {"id": "team/model", "name": "Team model",
            "context_window": 200_000, "max_tokens": 8192},
    });
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    // An Agent on the personal model default with no declared Settings was never saved, so its
    // frozen revision is zero; the rejections below each fail for their own field from that base.
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(command)) =
        &mut case.message
    else {
        panic!("fixture must be a session start")
    };
    if let Some(configuration) = command.payload.spec.agent_configuration.as_mut() {
        configuration.configuration_revision = 0;
    }
    case.wire["payload"]["spec"]["agent_configuration"]["configuration_revision"] = json!(0);
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    let configuration = "/payload/spec/agent_configuration";
    for (path, value) in [
        (
            format!("{configuration}/plugin_id"),
            json!("ora.1a2b/other"),
        ),
        (format!("{configuration}/plugin_version"), json!("1.2.1")),
        (format!("{configuration}/target"), json!("")),
        (format!("{configuration}/adapter_version"), json!(2)),
        (format!("{configuration}/settings_json"), json!("[1]")),
        (format!("{configuration}/settings_json"), json!("{")),
        (format!("{configuration}/model/id"), json!("")),
    ] {
        let mut wire = case.wire.clone();
        replace(&mut wire, &path, value);
        reject_semantics(
            Peer::Controller,
            &wire,
            &path,
            MessageValidationError::InvalidAgentConfiguration,
        )
        .await?;
    }
    // Without a binding an adapter could only fall back to whatever the host holds.
    let mut wire = case.wire.clone();
    wire["payload"]["spec"]
        .as_object_mut()
        .unwrap()
        .remove("model_binding_id");
    reject_semantics(
        Peer::Controller,
        &wire,
        "missing binding",
        MessageValidationError::InvalidAgentConfiguration,
    )
    .await?;
    let mut wire = case.wire.clone();
    replace(
        &mut wire,
        &format!("{configuration}/package_sha256"),
        json!("A".repeat(64)),
    );
    reject_semantics(
        Peer::Controller,
        &wire,
        "uppercase digest",
        MessageValidationError::InvalidSha256,
    )
    .await
}

/// Identity values that would corrupt a Git signature line and unbounded turns never cross.
#[tokio::test]
async fn start_agent_session_rejects_unsafe_identity_and_turns() -> Result<(), TestError> {
    let oversized = "x".repeat(MAX_USER_TURN_TEXT_BYTES + 1);
    let cases = [
        (
            "/payload/spec/git_identity/name",
            json!("Ada\nLovelace"),
            MessageValidationError::InvalidGitIdentity,
        ),
        (
            "/payload/spec/git_identity/name",
            json!("Ada <ada@example.com>"),
            MessageValidationError::InvalidGitIdentity,
        ),
        (
            "/payload/spec/git_identity/email",
            json!("ada example.com"),
            MessageValidationError::InvalidGitIdentity,
        ),
        (
            "/payload/spec/initial_turn/content",
            json!([]),
            MessageValidationError::InvalidUserTurn,
        ),
        (
            "/payload/spec/initial_turn/content/0/text",
            json!(oversized),
            MessageValidationError::InvalidUserTurn,
        ),
        (
            "/payload/spec/agent_plugin_id",
            json!("claude-code"),
            MessageValidationError::InvalidPluginId,
        ),
    ];
    for (path, value, expected) in cases {
        let mut wire = start().wire;
        replace(&mut wire, path, value);
        reject_semantics(Peer::Controller, &wire, path, expected).await?;
    }
    Ok(())
}

/// Thread events are ordinary sequenced events whose record stays opaque and bounded.
#[tokio::test]
async fn thread_events_round_trip_and_bound_the_record() -> Result<(), TestError> {
    let case = thread_event();
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.assert_envelope_rejections().await?;
    case.assert_fields(
        &["/sequence", "/payload/record"],
        &[
            ("/operation_id", "operation_id"),
            ("/execution_id", "execution_id"),
            ("/payload/turn_id", "turn_id"),
        ],
    )
    .await?;

    // A truncated record without a turn keeps the optional fields explicit on the wire.
    let mut truncated = thread_event();
    let Message::Node(NodeToControllerMessage::ThreadEvent(message)) = &mut truncated.message
    else {
        panic!("expected thread event")
    };
    message.payload.turn_id = None;
    message.payload.truncated = true;
    let payload = truncated.wire["payload"]
        .as_object_mut()
        .unwrap_or_else(|| panic!("payload must be an object"));
    payload.remove("turn_id");
    payload.insert("truncated".to_owned(), json!(true));
    truncated.assert_wire().await?;

    let mut projected = thread_event();
    let Message::Node(NodeToControllerMessage::ThreadEvent(message)) = &mut projected.message
    else {
        panic!("expected thread event")
    };
    message.payload.visible_text = Some("assistant answer".to_owned());
    projected.wire["payload"]["visible_text"] = json!("assistant answer");
    projected.assert_wire().await?;
    projected.assert_round_trip().await?;

    let mut oversized_projection = thread_event().wire;
    oversized_projection["payload"]["visible_text"] = json!("x".repeat(MAX_VISIBLE_TEXT_BYTES + 1));
    reject_semantics(
        Peer::Node,
        &oversized_projection,
        "oversized assistant text",
        MessageValidationError::VisibleTextTooLarge,
    )
    .await?;
    let mut missing_projection = thread_event().wire;
    missing_projection["payload"]["visible_text_truncated"] = json!(true);
    reject_semantics(
        Peer::Node,
        &missing_projection,
        "truncated assistant text with no text",
        MessageValidationError::VisibleTextTooLarge,
    )
    .await?;

    let mut wire = thread_event().wire;
    replace(&mut wire, "/payload/record", json!(["not", "an", "object"]));
    reject_structure(Peer::Node, &wire, "record must be an object").await;
    let mut huge = Map::new();
    huge.insert(
        "text".to_owned(),
        json!("x".repeat(MAX_THREAD_RECORD_BYTES)),
    );
    replace(&mut wire, "/payload/record", Value::Object(huge));
    reject_semantics(
        Peer::Node,
        &wire,
        "oversized record",
        MessageValidationError::ThreadRecordTooLarge,
    )
    .await
}

/// The terminal event and a Completed status decode as the session family.
#[tokio::test]
async fn session_end_round_trips_as_event_and_completed_status() -> Result<(), TestError> {
    let result = AgentSessionResult::AgentSessionEnded(AgentSessionEnded {
        node: node(),
        reason: AgentSessionEndReason::AgentFailed,
        detail: Some("agent_plugin_unavailable".to_owned()),
        scheduled_result: None,
        artifact_manifest: None,
    });
    let result_wire = json!({"kind": "agent_session_ended", "result": {
        "node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
        "reason": "agent_failed", "detail": "agent_plugin_unavailable"}});
    let event = Case {
        message: Message::Node(NodeToControllerMessage::AgentSessionEnded(
            AgentSessionEndedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                sequence: Sequence::new(/*value*/ 9),
                payload: result.clone(),
            },
        )),
        wire: json!({"message_type": "agent_session_ended", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION, "sequence": 9,
            "payload": result_wire}),
    };
    let status = Case {
        message: Message::Node(NodeToControllerMessage::ExecutionStatus(
            ExecutionStatusMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: ExecutionStatus {
                    stop_unconfirmed: Vec::new(),
                    node: node(),
                    state: ExecutionState::Completed(ExecutionResult::AgentSession(result)),
                },
            },
        )),
        wire: json!({"message_type": "execution_status", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
                "state": {"state": "completed", "result": result_wire}}}),
    };
    for case in [&event, &status] {
        case.assert_wire().await?;
        case.assert_round_trip().await?;
        case.assert_envelope_rejections().await?;
    }
    status.assert_historical_node().await?;
    let mut wire = event.wire;
    replace(
        &mut wire,
        "/payload/result/detail",
        json!("Error: agent crashed"),
    );
    reject_semantics(
        Peer::Node,
        &wire,
        "free-form detail",
        MessageValidationError::InvalidDetailCode,
    )
    .await
}

/// Session commands and their replies carry command identity but no event sequence.
#[tokio::test]
async fn session_commands_and_replies_round_trip() -> Result<(), TestError> {
    let envelope = |payload: Value, message_type: &str| {
        json!({"message_type": message_type, "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION, "payload": payload})
    };
    let cases = [
        Case {
            message: Message::Controller(ControllerToNodeMessage::SubmitUserTurn(
                SubmitUserTurnMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    payload: SubmitUserTurn {
                        node_id: NodeId::new("node-1"),
                        command_id: CommandId::new("command-1"),
                        turn: turn("turn-2", "Also update the changelog."),
                    },
                },
            )),
            wire: envelope(
                json!({"node_id": "node-1", "command_id": "command-1", "turn": {"turn_id": "turn-2",
                    "content": [{"type": "text", "text": "Also update the changelog."}]}}),
                "submit_user_turn",
            ),
        },
        Case {
            message: Message::Controller(ControllerToNodeMessage::EndSession(EndSessionMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: EndSession {
                    node_id: NodeId::new("node-1"),
                    command_id: CommandId::new("command-2"),
                    reason: EndSessionReason::IdleTimeout,
                },
            })),
            wire: envelope(
                json!({"node_id": "node-1", "command_id": "command-2", "reason": "idle_timeout"}),
                "end_session",
            ),
        },
        Case {
            message: Message::Node(NodeToControllerMessage::SessionCommandAccepted(
                SessionCommandAcceptedMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    payload: SessionCommandAccepted {
                        command_id: CommandId::new("command-1"),
                    },
                },
            )),
            wire: envelope(
                json!({"command_id": "command-1"}),
                "session_command_accepted",
            ),
        },
        Case {
            message: Message::Node(NodeToControllerMessage::SessionCommandRejected(
                SessionCommandRejectedMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    payload: SessionCommandRejected {
                        command_id: CommandId::new("command-2"),
                        reason: SessionCommandRejection::SessionEnded,
                    },
                },
            )),
            wire: envelope(
                json!({"command_id": "command-2", "reason": "session_ended"}),
                "session_command_rejected",
            ),
        },
    ];
    for case in &cases {
        case.assert_wire().await?;
        case.assert_round_trip().await?;
        case.assert_envelope_rejections().await?;
        case.assert_fields(
            &["/payload/command_id"],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/command_id", "command_id"),
            ],
        )
        .await?;
    }
    Ok(())
}

/// Runtime-control dispatch preserves the original session input and rejects authority for another target.
#[tokio::test]
async fn controlled_session_start_round_trips_and_binds_exact_identity() -> Result<(), TestError> {
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(command)) = start().message
    else {
        unreachable!()
    };
    let binding = RuntimeBinding {
        tenant_id: "tenant".into(),
        workspace_id: "workspace".into(),
        sandbox_id: "sandbox".into(),
        runtime_generation: 1,
        node_id: command.payload.spec.node_id.as_str().into(),
        node_incarnation_id: "incarnation".into(),
        node_instance_id: "instance".into(),
        controller_epoch: 1,
        control_epoch: 1,
        control_version: 1,
        session_id: "session".into(),
        actor_user_id: "actor".into(),
        operation_id: String::new(),
        execution_id: command.execution_id.as_str().into(),
        node_operation_id: command.operation_id.as_str().into(),
        input_closed: false,
        issued_at_ms: 1000,
        expires_at_ms: 31_000,
    };
    let envelope = ControlledStartAgentSession { binding, command };
    let message = ControllerToNodeMessage::ControlledStartAgentSession(Box::new(envelope.clone()));
    assert!(message.validate().is_ok());
    pretty_assertions::assert_eq!(round_trip_controller(message.clone()).await?, message);
    let mut changed = envelope.clone();
    changed.binding.node_operation_id = "wrong".into();
    assert!(changed.validate().is_err());
    changed = envelope.clone();
    changed.binding.execution_id = "wrong".into();
    assert!(changed.validate().is_err());
    changed = envelope.clone();
    changed.binding.node_id = "wrong".into();
    assert!(changed.validate().is_err());
    changed = envelope;
    changed.binding.input_closed = true;
    assert!(changed.validate().is_err());
    Ok(())
}

/// A scheduled session declares how Cloud admitted it; a manual run leaves the field absent, which
/// is also what every session released before Cloud admitted scheduled occurrences looks like.
#[tokio::test]
async fn a_scheduled_session_start_carries_its_trigger() -> Result<(), TestError> {
    let mut case = start();
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(command)) =
        &mut case.message
    else {
        panic!("fixture must be a session start")
    };
    command.payload.spec.trigger = Some(SessionTrigger::Scheduled);
    case.wire["payload"]["spec"]["trigger"] = json!("scheduled");

    case.assert_wire().await?;
    case.assert_round_trip().await?;
    assert_eq!(start().wire["payload"]["spec"].get("trigger"), None);
    Ok(())
}

/// The terminal result of a scheduled run carries the verdict and the artifact declaration it made
/// before its Workspace was released; Cloud verifies both and never parses the Thread for them.
#[tokio::test]
async fn a_scheduled_terminal_result_declares_its_verdict_and_artifacts() -> Result<(), TestError> {
    let result = AgentSessionResult::AgentSessionEnded(AgentSessionEnded {
        node: node(),
        reason: AgentSessionEndReason::IdleTimeout,
        detail: None,
        scheduled_result: Some(ScheduledRunResult {
            verdict: ScheduledVerdict::RangeUnresolved,
            verdict_detail: Some("no comparison base".to_owned()),
        }),
        artifact_manifest: Some(ScheduledArtifactManifest {
            items: vec![ScheduledArtifactItem {
                index: 0,
                display_name: "reports/review.md".to_owned(),
                kind: ScheduledArtifactKind::Markdown,
                size_bytes: 5,
                sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                    .to_owned(),
            }],
            rejections: vec![ScheduledArtifactRejection {
                display_name: "notes.pdf".to_owned(),
                reason: ScheduledArtifactRejectionReason::UnsupportedType,
            }],
            omitted_rejections: 0,
            walk_truncated: false,
        }),
    });
    let case = Case {
        message: Message::Node(NodeToControllerMessage::AgentSessionEnded(
            AgentSessionEndedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                sequence: Sequence::new(/*value*/ 4),
                payload: result.clone(),
            },
        )),
        wire: json!({"message_type": "agent_session_ended", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION, "sequence": 4,
            "payload": {"kind": "agent_session_ended", "result": {
                "node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
                "reason": "idle_timeout",
                "scheduled_result": {"verdict": "range_unresolved",
                    "verdict_detail": "no comparison base"},
                "artifact_manifest": {
                    "items": [{"index": 0, "display_name": "reports/review.md",
                        "kind": "markdown", "size_bytes": 5,
                        "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}],
                    "rejections": [{"display_name": "notes.pdf", "reason": "unsupported_type"}]}}}}),
    };
    case.assert_wire().await?;
    case.assert_round_trip().await?;

    let mut over_long = case.wire.clone();
    over_long["payload"]["result"]["scheduled_result"]["verdict_detail"] =
        json!("x".repeat(MAX_SCHEDULED_VERDICT_DETAIL_CHARS + 1));
    reject_semantics(
        Peer::Node,
        &over_long,
        "over-long verdict detail",
        MessageValidationError::InvalidScheduledResult,
    )
    .await?;

    let mut escaping = case.wire.clone();
    escaping["payload"]["result"]["artifact_manifest"]["items"][0]["display_name"] =
        json!("../escape.md");
    reject_semantics(
        Peer::Node,
        &escaping,
        "unsafe artifact path",
        MessageValidationError::InvalidScheduledArtifactManifest,
    )
    .await?;

    // The collector folds names with Unicode case folding after NFC, so two spellings one
    // case-insensitive filesystem treats as one name must not both be declared.
    let mut unicode_case = case.wire.clone();
    unicode_case["payload"]["result"]["artifact_manifest"]["items"] = json!([
        {"index": 0, "display_name": "Á.txt", "kind": "text", "size_bytes": 1,
            "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"},
        {"index": 1, "display_name": "á.txt", "kind": "text", "size_bytes": 1,
            "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}
    ]);
    reject_semantics(
        Peer::Node,
        &unicode_case,
        "one path in two Unicode cases",
        MessageValidationError::InvalidScheduledArtifactManifest,
    )
    .await?;

    // Combining and precomposed spellings are the same path too.
    let mut unicode_form = case.wire.clone();
    unicode_form["payload"]["result"]["artifact_manifest"]["items"] = json!([
        {"index": 0, "display_name": "é.txt", "kind": "text", "size_bytes": 1,
            "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"},
        {"index": 1, "display_name": "e\u{301}.txt", "kind": "text", "size_bytes": 1,
            "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}
    ]);
    reject_semantics(
        Peer::Node,
        &unicode_form,
        "one path in two normalization forms",
        MessageValidationError::InvalidScheduledArtifactManifest,
    )
    .await?;

    // A manifest lists at most the bounded number of refusals; further refusals are counted in
    // `omitted_rejections` instead of inflating the terminal result.
    let mut too_many = case.wire.clone();
    too_many["payload"]["result"]["artifact_manifest"]["rejections"] = Value::Array(
        (0..=MAX_SCHEDULED_ARTIFACT_REJECTIONS)
            .map(|index| {
                json!({"display_name": format!("reject-{index}.pdf"),
                "reason": "unsupported_type"})
            })
            .collect(),
    );
    reject_semantics(
        Peer::Node,
        &too_many,
        "more refusals than one manifest lists",
        MessageValidationError::InvalidScheduledArtifactManifest,
    )
    .await?;

    // A walk that stopped at its candidate bound is a fact the terminal event carries on its own, so
    // a Controller sees that candidates were never examined even when the refusal list is full.
    let mut cut = case.wire.clone();
    cut["payload"]["result"]["artifact_manifest"]["rejections"] = Value::Array(
        (0..MAX_SCHEDULED_ARTIFACT_REJECTIONS)
            .map(|index| {
                json!({"display_name": format!("reject-{index}.pdf"),
                "reason": "unsupported_type"})
            })
            .collect(),
    );
    cut["payload"]["result"]["artifact_manifest"]["omitted_rejections"] = json!(448);
    cut["payload"]["result"]["artifact_manifest"]["walk_truncated"] = json!(true);
    let decoded: NodeToControllerMessage = serde_json::from_value(cut)?;
    assert_eq!(round_trip_node(decoded.clone()).await?, decoded);
    Ok(())
}

/// Refusals beyond the listed bound stay visible as a count, and the count is absent when none
/// were dropped.
#[test]
fn a_manifest_summarises_refusals_beyond_its_list_bound() {
    assert_eq!(
        serde_json::to_value(ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: Vec::new(),
            omitted_rejections: 12,
            walk_truncated: false,
        })
        .expect("a manifest serializes"),
        json!({"items": [], "rejections": [], "omitted_rejections": 12})
    );
    assert_eq!(
        serde_json::to_value(ScheduledArtifactManifest::default()).expect("a manifest serializes"),
        json!({"items": [], "rejections": []})
    );
}

/// A cut walk states itself on the wire, even when the refusal list is already full and cannot
/// carry the detail; a complete walk leaves the fact absent.
#[test]
fn a_manifest_states_a_cut_walk_separately_from_its_refusals() {
    let manifest = ScheduledArtifactManifest {
        items: Vec::new(),
        rejections: vec![ScheduledArtifactRejection {
            display_name: "nested".to_owned(),
            reason: ScheduledArtifactRejectionReason::CandidateLimit,
        }],
        omitted_rejections: 448,
        walk_truncated: true,
    };

    assert_eq!(
        serde_json::to_value(&manifest).expect("a manifest serializes"),
        json!({
            "items": [],
            "rejections": [{"display_name": "nested", "reason": "candidate_limit"}],
            "omitted_rejections": 448,
            "walk_truncated": true,
        })
    );
    manifest.validate().expect("a cut walk is reportable");
    assert_eq!(
        serde_json::to_value(&ScheduledArtifactManifest::default())
            .expect("a manifest serializes")
            .get("walk_truncated"),
        None
    );
}

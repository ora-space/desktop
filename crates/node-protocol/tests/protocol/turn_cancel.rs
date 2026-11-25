//! `CancelTurn`, its settlement, and the `turn_cancel` handshake.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md cases 17 and 20:
//! new shapes round trip, and peers that never opt in keep the exact previous wire shapes.
use super::support::*;
use ora_node_protocol::*;
use serde_json::{Value, json};

const OPERATION: &str = "run-1";
const EXECUTION: &str = "execution-session";

/// Wraps a payload in a session-command envelope.
fn envelope(message_type: &str, payload: Value) -> Value {
    json!({"message_type": message_type, "protocol_version": 1,
        "operation_id": OPERATION, "execution_id": EXECUTION, "payload": payload})
}

/// The cancel command and the settlement round trip with their exact wire names.
#[tokio::test]
async fn cancel_and_settlement_round_trip() -> Result<(), TestError> {
    let cancel = Case {
        message: Message::Controller(ControllerToNodeMessage::CancelTurn(CancelTurnMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new(OPERATION),
            execution_id: ExecutionId::new(EXECUTION),
            payload: CancelTurn {
                node_id: NodeId::new("node-1"),
                command_id: CommandId::new("cancel-1"),
                turn_id: TurnId::new("turn-2"),
            },
        })),
        wire: envelope(
            "cancel_turn",
            json!({"node_id": "node-1", "command_id": "cancel-1", "turn_id": "turn-2"}),
        ),
    };
    cancel.assert_wire().await?;
    cancel.assert_round_trip().await?;
    cancel.assert_envelope_rejections().await?;
    cancel
        .assert_fields(
            &["/payload/command_id", "/payload/turn_id"],
            &[
                ("/payload/node_id", "node_id"),
                ("/payload/command_id", "command_id"),
                ("/payload/turn_id", "turn_id"),
            ],
        )
        .await?;
    for (outcome, name) in [
        (TurnCancelOutcome::Stopped, "stopped"),
        (TurnCancelOutcome::NotStarted, "not_started"),
        (TurnCancelOutcome::AlreadyEnded, "already_ended"),
        (TurnCancelOutcome::Unknown, "unknown"),
        (TurnCancelOutcome::SessionEnding, "session_ending"),
    ] {
        let mut wire = envelope(
            "turn_cancel_settled",
            json!({"command_id": "cancel-1", "turn_id": "turn-2", "outcome": name}),
        );
        wire["sequence"] = json!(7);
        let settled = Case {
            message: Message::Node(NodeToControllerMessage::TurnCancelSettled(
                TurnCancelSettledMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    sequence: Sequence::new(7),
                    payload: TurnCancelSettled {
                        command_id: CommandId::new("cancel-1"),
                        turn_id: TurnId::new("turn-2"),
                        outcome,
                    },
                },
            )),
            wire,
        };
        settled.assert_wire().await?;
        settled.assert_round_trip().await?;
    }
    let rejected = Case {
        message: Message::Node(NodeToControllerMessage::SessionCommandRejected(
            SessionCommandRejectedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: SessionCommandRejected {
                    command_id: CommandId::new("cancel-1"),
                    reason: SessionCommandRejection::TurnCancelUnsupported,
                },
            },
        )),
        wire: envelope(
            "session_command_rejected",
            json!({"command_id": "cancel-1", "reason": "turn_cancel_unsupported"}),
        ),
    };
    rejected.assert_wire().await?;
    rejected.assert_round_trip().await
}

/// The confirmation and the capability travel together, and only beside session support.
#[tokio::test]
async fn turn_cancel_negotiation_requires_its_capability() -> Result<(), TestError> {
    let node = super::session::node();
    let accepted = |turn_cancel: bool, capabilities: Vec<NodeCapability>| {
        NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: HelloAccepted {
                thread_previews: false,
                turn_cancel,
                turn_failure: false,
                selected_version: CURRENT_PROTOCOL_VERSION,
                node: node.clone(),
                capabilities,
                model_adapter: None,
            },
        })
    };
    let negotiated = accepted(
        true,
        vec![NodeCapability::AgentSession, NodeCapability::TurnCancel],
    );
    assert_eq!(negotiated.validate(), Ok(()));
    assert_eq!(round_trip_node(negotiated.clone()).await?, negotiated);
    for message in [
        accepted(true, vec![NodeCapability::AgentSession]),
        accepted(
            false,
            vec![NodeCapability::AgentSession, NodeCapability::TurnCancel],
        ),
        accepted(
            true,
            vec![
                NodeCapability::WorktreeExecution,
                NodeCapability::TurnCancel,
            ],
        ),
    ] {
        assert_eq!(
            message.validate(),
            Err(MessageValidationError::InvalidTurnCancelNegotiation)
        );
    }
    Ok(())
}

/// Sessions that choose neither keep behaviour encode exactly as before the fields existed.
#[test]
fn unchosen_keep_fields_are_omitted() {
    let start = super::agent_session::start();
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(mut message)) =
        start.message
    else {
        unreachable!("a session start");
    };
    let encoded = serde_json::to_value(&message.payload.spec).unwrap();
    assert!(encoded.get("keep_session_on_cancel").is_none());
    assert!(encoded.get("keep_session_on_failure").is_none());
    message.payload.spec.keep_session_on_cancel = true;
    let encoded = serde_json::to_value(&message.payload.spec).unwrap();
    assert_eq!(encoded["keep_session_on_cancel"], json!(true));
    assert!(encoded.get("keep_session_on_failure").is_none());
}

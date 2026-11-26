//! The `turn_failure` capability, its negotiation and `TurnFailureSettled`.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md case 13: the settlement
//! round trips with its wire names, and the capability is confirmed only beside a confirmed
//! `turn_cancel`, with any asymmetry rejected.

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

/// Case 13: the settlement round trips and names each outcome with its exact wire name.
#[tokio::test]
async fn failure_settlement_round_trips_with_its_wire_names() -> Result<(), TestError> {
    for (outcome, name) in [
        (TurnFailureOutcome::Kept, "kept"),
        (TurnFailureOutcome::SessionEnding, "session_ending"),
    ] {
        let mut wire = envelope(
            "turn_failure_settled",
            json!({"turn_id": "turn-2", "outcome": name}),
        );
        wire["sequence"] = json!(9);
        let settled = Case {
            message: Message::Node(NodeToControllerMessage::TurnFailureSettled(
                TurnFailureSettledMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    sequence: Sequence::new(9),
                    payload: TurnFailureSettled {
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
    Ok(())
}

/// Case 13: `turn_failure` is confirmed only beside a confirmed `turn_cancel` and its capability;
/// any asymmetry between the confirmation and the capability is rejected.
#[tokio::test]
async fn turn_failure_negotiation_requires_turn_cancel_and_its_capability() -> Result<(), TestError>
{
    let node = super::session::node();
    let accepted = |turn_cancel: bool, turn_failure: bool, capabilities: Vec<NodeCapability>| {
        NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: HelloAccepted {
                thread_previews: false,
                turn_cancel,
                turn_failure,
                selected_version: CURRENT_PROTOCOL_VERSION,
                node: node.clone(),
                capabilities,
                model_adapter: None,
            },
        })
    };
    let negotiated = accepted(
        /*turn_cancel*/ true,
        /*turn_failure*/ true,
        vec![
            NodeCapability::AgentSession,
            NodeCapability::TurnCancel,
            NodeCapability::TurnFailure,
        ],
    );
    assert_eq!(negotiated.validate(), Ok(()));
    assert_eq!(round_trip_node(negotiated.clone()).await?, negotiated);
    for message in [
        // The confirmation without its capability.
        accepted(
            /*turn_cancel*/ true,
            /*turn_failure*/ true,
            vec![NodeCapability::AgentSession, NodeCapability::TurnCancel],
        ),
        // The capability without its confirmation.
        accepted(
            /*turn_cancel*/ true,
            /*turn_failure*/ false,
            vec![
                NodeCapability::AgentSession,
                NodeCapability::TurnCancel,
                NodeCapability::TurnFailure,
            ],
        ),
        // Confirmed failure beside its capability but without `turn_cancel`.
        accepted(
            /*turn_cancel*/ false,
            /*turn_failure*/ true,
            vec![NodeCapability::AgentSession, NodeCapability::TurnFailure],
        ),
    ] {
        assert_eq!(
            message.validate(),
            Err(MessageValidationError::InvalidTurnFailureNegotiation)
        );
    }
    Ok(())
}

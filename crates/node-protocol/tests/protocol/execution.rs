use super::session::node;
use super::support::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[path = "execution/fixtures.rs"]
mod fixtures;
#[path = "execution/rejections.rs"]
mod rejections;

/// Keeps every occurrence of an identity in the same transparent JSON representation.
#[test]
fn serializes_identity_consistently_across_messages() -> Result<(), serde_json::Error> {
    let operation_id = OperationId::new("operation-1");
    let controller = ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: operation_id.clone(),
        execution_id: ExecutionId::new("execution-1"),
        payload: GetExecutionStatus {
            node_id: NodeId::new("node-1"),
        },
    });
    let node = NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id,
        execution_id: ExecutionId::new("execution-1"),
        payload: ExecutionStatus {
            node: node(),
            state: ExecutionState::Running,
        },
    });

    let controller_json = serde_json::to_value(controller)?;
    let node_json = serde_json::to_value(node)?;
    assert_eq!(controller_json["operation_id"], node_json["operation_id"]);
    Ok(())
}

/// Proves fragmented I/O preserves every legal message in this business slice.
#[tokio::test]
async fn round_trips_messages() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_round_trip().await?;
    }
    Ok(())
}

/// Locks independent wire shapes, optional output fields, and extension acceptance.
#[tokio::test]
async fn preserves_wire_shapes() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_wire().await?;
        case.assert_extensions().await?;
    }
    Ok(())
}

/// Applies version and direction constraints to every legal shape in this slice.
#[tokio::test]
async fn rejects_versions_directions_and_payloads() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_envelope_rejections().await?;
    }
    Ok(())
}

/// Rejects illegal state tags and result combinations before semantic validation.
#[tokio::test]
async fn rejects_structural_state_contradictions() -> Result<(), TestError> {
    let case = fixtures::execution_status_running();
    receive(case.peer(), &case.wire).await?;
    for state in [
        json!({"state":"completed"}),
        json!({"state":"unknown","result":{}}),
        json!({"state":"accepted","result":{}}),
        json!({"state":"running","result":{}}),
        json!({"state":"invalid"}),
        json!({"state":"completed","result":{"kind":"invalid","result":{}}}),
    ] {
        let mut wire = case.wire.clone();
        replace(&mut wire, "/payload/state", state);
        reject_structure(case.peer(), &wire, "/payload/state").await;
    }
    Ok(())
}

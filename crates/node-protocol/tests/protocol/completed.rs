use super::*;
use pretty_assertions::assert_eq;

/// Exercises all terminal variants through both public validation paths without losing history.
#[tokio::test]
async fn completed_results_preserve_incarnations_and_reject_other_nodes() -> Result<(), TestError> {
    for result in [
        WorktreeExecutionResult::Ready(ready_result()),
        WorktreeExecutionResult::Failed(failed_result()),
        WorktreeExecutionResult::Removed(removed_result()),
        WorktreeExecutionResult::RemovalFailed(removal_failed_result()),
    ] {
        let historical = NodeToControllerMessage::ExecutionStatus {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-1"),
            execution_id: ExecutionId::new("execution-1"),
            payload: ExecutionStatus {
                node: NodeRuntimeIdentity {
                    node_id: NodeId::new("node-1"),
                    incarnation_id: NodeIncarnationId::new("incarnation-2"),
                },
                state: ExecutionState::Completed(result),
            },
        };
        assert_eq!(round_trip_node(historical.clone()).await?, historical);

        // Construct wire bytes directly so outbound validation cannot mask a receive regression.
        let mut wire = serde_json::to_value(historical)?;
        wire["payload"]["node"]["node_id"] = json!("node-2");
        let bytes = framed(NODE_MESSAGE_FRAME_TYPE, &serde_json::to_vec(&wire)?)?;
        let expected = MessageValidationError::CompletedNodeMismatch {
            reporter: NodeId::new("node-2"),
            result: NodeId::new("node-1"),
        };
        let received = read_node_message(&mut bytes.as_slice()).await;
        match received {
            Err(FrameError::InvalidMessage(error)) => assert_eq!(error, expected),
            other => panic!("expected Node identity conflict, got {other:?}"),
        }
        let invalid = serde_json::from_value(wire)?;
        let mut output = Vec::new();
        match write_node_message(&mut output, &invalid).await {
            Err(FrameError::InvalidMessage(error)) => assert_eq!(error, expected),
            other => panic!("expected Node identity conflict, got {other:?}"),
        }
        assert_eq!(output, Vec::<u8>::new());
    }
    Ok(())
}

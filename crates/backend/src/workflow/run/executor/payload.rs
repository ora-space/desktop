use super::{AgentNodeOutcome, NodeExecutionError};
use agent_client_protocol_schema::v1::StopReason;
use ora_application::{NodeFailure, NodeFailureKind, WorkflowRunCallback, WorkflowRunPayload};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId};
use std::sync::Arc;

/// Parses the immutable locale and skill placement receipt captured when the run was created.
pub(super) fn parse_workflow_run_payload(
    payload: Option<&str>,
) -> Result<WorkflowRunPayload, NodeExecutionError> {
    payload
        .and_then(|payload| serde_json::from_str(payload).ok())
        .ok_or(NodeExecutionError::InvalidRunPayload)
}

/// Reports one finished turn to the engine according to the confirmed stop-reason mapping.
pub(super) fn report_outcome(
    callback: &Arc<dyn WorkflowRunCallback>,
    run_id: &WorkflowRunId,
    node_run_id: &WorkflowNodeRunId,
    outcome: AgentNodeOutcome,
) {
    let AgentNodeOutcome::Completed {
        output,
        structured_output,
        stop_reason,
        file_changes,
    } = outcome
    else {
        // An interactive node parked at `Pending` reports nothing; the human drives completion.
        return;
    };
    match stop_reason {
        StopReason::EndTurn => callback.complete_node(
            run_id,
            node_run_id,
            output,
            structured_output,
            Some("end_turn".to_string()),
            file_changes,
        ),
        StopReason::MaxTokens => callback.complete_node(
            run_id,
            node_run_id,
            output,
            structured_output,
            Some("max_tokens".to_string()),
            file_changes,
        ),
        StopReason::MaxTurnRequests => callback.complete_node(
            run_id,
            node_run_id,
            output,
            structured_output,
            Some("max_turn_requests".to_string()),
            file_changes,
        ),
        StopReason::Refusal => callback.fail_node(
            run_id,
            node_run_id,
            NodeFailure::new(NodeFailureKind::AgentRefusal, "agent refused the request")
                .with_output(output),
        ),
        StopReason::Cancelled => {
            // Non-interactive cancellation belongs to the run cancel flow; interactive turns
            // already returned through the awaiting-input branch above.
        }
        // A newer ACP stop reason has semantics this executor cannot safely map to a
        // successful workflow transition.
        _ => callback.fail_node(
            run_id,
            node_run_id,
            NodeFailure::new(
                NodeFailureKind::UnknownStopReason,
                "agent stopped for a reason this Ora version does not recognize",
            )
            .with_output(output),
        ),
    }
}

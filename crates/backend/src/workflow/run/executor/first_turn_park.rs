//! First-turn parking for an interactive Agent node.
//!
//! The park is deliberately separate from the node driver: reading the whole session history is
//! file I/O and the transition sink takes the per-run gate, so both run on a blocking worker and
//! the snapshot is derived before the gate is taken (ADR D3 steps 1 and 4).

use super::super::completion_snapshot::{
    StopCause, TurnOrigin, completion_payload, derive_completion_snapshot,
    unknown_completion_snapshot, with_file_check_warning,
};
use super::super::transitions::WorkflowRunTransitions;
use super::NodeExecutionError;
use ora_application::{AgentCompletion, AgentOutputContract, RepositoryError};
use ora_domain::{SessionId, WorkflowNodeRunId, WorkflowRunId};
use ora_history::read_session_history;
use ora_logging::ora_warn;
use std::path::PathBuf;
use std::sync::Arc;

/// The frozen facts one first-turn park needs, grouped so the park entry stays readable.
pub(super) struct FirstTurnPark {
    pub sessions_root: PathBuf,
    pub run_id: WorkflowRunId,
    pub node_run_id: WorkflowNodeRunId,
    pub session_id: SessionId,
    pub output_contract: Option<AgentOutputContract>,
    pub completion: Option<AgentCompletion>,
    pub baseline_available: bool,
    pub stop_cause: Option<StopCause>,
}

/// Drives one interactive node's first-turn park off the async runtime.
pub(super) async fn park_first_interactive_turn(
    transitions: Arc<WorkflowRunTransitions>,
    park: FirstTurnPark,
) -> Result<(), NodeExecutionError> {
    let FirstTurnPark {
        sessions_root,
        run_id,
        node_run_id,
        session_id,
        output_contract,
        completion,
        baseline_available,
        stop_cause,
    } = park;
    tokio::task::spawn_blocking(move || {
        let snapshot = with_file_check_warning(
            match read_session_history(&sessions_root, session_id.as_ref()) {
                Ok(history) => derive_completion_snapshot(
                    &history,
                    output_contract.as_ref(),
                    false,
                    TurnOrigin::Workflow,
                    stop_cause,
                ),
                Err(error) => {
                    ora_warn!(
                        node_run_id = %node_run_id,
                        error = %error,
                        "failed to read session history while parking; recording an unknown completion snapshot"
                    );
                    unknown_completion_snapshot(
                        false,
                        TurnOrigin::Workflow,
                        Some(StopCause::Unknown),
                    )
                }
            },
            completion.as_ref(),
            baseline_available,
        );
        transitions
            .park_node_run(&run_id, &node_run_id, &completion_payload(&snapshot))
            .map_err(NodeExecutionError::from)
    })
    .await
    .map_err(|source| {
        NodeExecutionError::Repository(RepositoryError::from_message(format!(
            "interactive park did not complete: {source}"
        )))
    })?
    .map(|_advanced| ())
}

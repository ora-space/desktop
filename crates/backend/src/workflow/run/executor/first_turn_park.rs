//! First-turn parking for an interactive Agent node.
//!
//! The park is deliberately separate from the node driver: reading the whole session history is
//! file I/O and the transition sink takes the per-run gate, so both run on a blocking worker and
//! the snapshot is derived before the gate is taken (ADR D3 steps 1 and 4).

use super::super::completion_snapshot::{
    CompletionMeta, CompletionPolicy, StopCause, TurnOrigin, completion_payload_with_meta,
    derive_completion_snapshot, unknown_completion_snapshot, with_file_check_warning,
};
use super::super::interactive::AgentSignalStore;
use super::super::transitions::WorkflowRunTransitions;
use super::NodeExecutionError;
use ora_application::{
    AgentCompletion, AgentCompletionSignal, AgentOutputContract, RepositoryError,
};
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
    pub completion_signal: AgentCompletionSignal,
    pub agent_signals: Arc<AgentSignalStore>,
    pub attempt_id: Option<String>,
    pub auto_accept: bool,
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
        completion_signal,
        agent_signals,
        attempt_id,
        auto_accept,
        baseline_available,
        stop_cause,
    } = park;
    tokio::task::spawn_blocking(move || {
        let policy = match completion_signal {
            AgentCompletionSignal::Agent => CompletionPolicy::Agent,
            AgentCompletionSignal::EndTurn => CompletionPolicy::EndTurn,
        };
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
        // Bind only the settlement this attempt produced; a late tool call after the turn changed
        // cannot attach its signal to a newer turn.
        let agent_signal = attempt_id.as_deref().and_then(|attempt| {
            let turn_key = snapshot.turn_key?;
            agent_signals.bind(&format!("{node_run_id}"), attempt, turn_key)
        });
        if let Some(attempt) = attempt_id.as_deref() {
            agent_signals.revoke_attempt(&format!("{node_run_id}"), attempt);
        }
        let meta = CompletionMeta {
            policy,
            agent_signal,
            confirmation: None,
            auto_accept,
        };
        let snapshot = snapshot.with_agent_signal_gate(&meta);
        transitions
            .park_node_run(&run_id, &node_run_id, &completion_payload_with_meta(&snapshot, &meta))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::run::completion_snapshot::{
        CompletionBlockerKind, FILE_CHECK_UNAVAILABLE, parse_completion_snapshot,
    };
    use crate::workflow::run::test_fixture::{
        bootstrap, recording_transitions, run_test, started_run, write_settled_turn,
    };
    use ora_application::{GetWorkflowRunHandler, WorkflowGraph};
    use ora_contracts::{GetWorkflowRunRequest, WorkflowNodeStatus as ContractNodeStatus};
    use ora_db::SqliteWorkflowRunRepository;
    use ora_domain::{SessionId, WorkflowNodeStatus};
    use std::sync::Arc;

    const FIRST_TURN_GRAPH: &str = r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"reports/keep.md","mustChange":true}]}}}}
    ],"edges":[{"source":"start","target":"agent"}]}"#;

    /// The production first-turn park records the missing-baseline warning in the read projection.
    #[test]
    fn missing_baseline_warning_is_visible_after_the_first_turn_park() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let (run_id, node_runs) = started_run(&temp, &pool, FIRST_TURN_GRAPH);
            let agent = node_runs
                .iter()
                .find(|node| node.node_id == "agent")
                .expect("agent node");
            assert_eq!(agent.status, WorkflowNodeStatus::Running);
            let sessions_root = temp.path().join("sessions");
            let baseline_path = temp
                .path()
                .join("baselines")
                .join(format!("{}.json", agent.id));
            assert!(
                !baseline_path.exists(),
                "the warning must describe a physically missing baseline"
            );

            write_settled_turn(&sessions_root, "session-1", "go", "done");
            let graph = WorkflowGraph::parse(FIRST_TURN_GRAPH).expect("parse graph");
            let completion = graph
                .node("agent")
                .and_then(|node| node.agent_config.as_ref())
                .and_then(|config| config.completion.clone());
            let (transitions, _recording) = recording_transitions(&pool);
            park_first_interactive_turn(
                transitions,
                FirstTurnPark {
                    sessions_root: sessions_root.clone(),
                    run_id: run_id.clone(),
                    node_run_id: agent.id.clone(),
                    session_id: SessionId::new("session-1"),
                    output_contract: None,
                    completion,
                    completion_signal: AgentCompletionSignal::EndTurn,
                    agent_signals: Arc::new(AgentSignalStore::new()),
                    attempt_id: None,
                    auto_accept: false,
                    baseline_available: false,
                    stop_cause: None,
                },
            )
            .await
            .expect("first turn parks through the production transition sink");

            // Read through the same get projection Desktop uses, not just the persisted payload.
            let projection = GetWorkflowRunHandler::new(Arc::new(
                SqliteWorkflowRunRepository::new(pool.clone()),
            ));
            let detail = projection
                .handle(GetWorkflowRunRequest {
                    run_id: run_id.to_string(),
                })
                .expect("run detail");
            let node = detail
                .nodes
                .iter()
                .find(|node| node.node_id == "agent")
                .expect("projected agent node");
            assert_eq!(node.status, ContractNodeStatus::Pending);
            let snapshot =
                parse_completion_snapshot(node.payload.as_deref()).expect("park snapshot");
            assert_eq!(snapshot.turn_origin, Some(TurnOrigin::Workflow));
            assert!(
                snapshot
                    .blockers
                    .iter()
                    .any(|blocker| blocker.kind == CompletionBlockerKind::TurnUnconfirmed),
                "an EndTurn opening turn still waits for explicit confirmation"
            );
            assert_eq!(
                snapshot
                    .warnings
                    .iter()
                    .filter(|warning| *warning == FILE_CHECK_UNAVAILABLE)
                    .count(),
                1,
                "the missing mustChange baseline warns once in the projection snapshot"
            );
        });
    }
}

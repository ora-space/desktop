//! The end the runtime attributed to a workflow turn must reach the parked snapshot.
//!
//! `turns` is where the runtime's `PromptEndReport` becomes the park's `StopCause`; driving the
//! same function the stream's cleanup closure calls proves that wiring, rather than only proving
//! that `end_human_turn` classifies a cause handed to it directly.

use super::session::begin_human_turn;
use super::turns::park_ended_turn;
use crate::workflow::run::completion_snapshot::{
    CompletionBlocker, CompletionBlockerKind, CompletionSnapshot, ParkKind, StopCause, TurnKey,
    TurnOrigin, parse_completion_snapshot,
};
use crate::workflow::run::test_fixture::{
    INTERACTIVE_AGENT_GRAPH, bind_and_park, bootstrap, end_fixture_turn, locks,
    recording_transitions, run_test, started_run, write_open_turn,
};
use agent_client_protocol_schema::v1::StopReason;
use ora_agent_runtime::{PromptEndCause, PromptEndReport};
use ora_application::WorkflowRunRepository;
use ora_db::{RepositoryPool, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId};
use ora_utils::hash::sha256_hex;
use pretty_assertions::assert_eq;
use serde_json::Value;

/// Returns the named node's persisted row so its parked status and snapshot assert together.
fn parked_node(pool: &RepositoryPool, run_id: &WorkflowRunId, node_id: &str) -> WorkflowNodeRun {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
        .into_iter()
        .find(|node| node.node_id == node_id)
        .expect("agent node run")
}

/// The prompt cleanup parks a timeout, a dropped stream and a crash under their own cause.
///
/// Each iteration records the cause on the same kind of report the actor fills while it settles a
/// real turn, then runs the exact function the prompt's stream attaches as cleanup. The persisted
/// snapshot — which the read projection and the completion gate consume — must keep the cause and
/// classify the park as abnormal instead of flattening all three into `Cancelled`.
#[test]
fn a_workflow_turn_records_the_reported_end_cause_in_its_parked_snapshot() {
    run_test(async {
        for (end, expected) in [
            (PromptEndCause::InactivityTimeout, StopCause::PromptTimedOut),
            (PromptEndCause::StreamDropped, StopCause::StreamDropped),
            (PromptEndCause::SessionCrashed, StopCause::SessionCrashed),
        ] {
            let (temp, pool) = bootstrap();
            let (run_id, node_runs) = started_run(&temp, &pool, INTERACTIVE_AGENT_GRAPH);
            let agent = node_runs
                .iter()
                .find(|node| node.node_id == "agent")
                .unwrap();
            let (session_id, node_run_id) = bind_and_park(&pool, agent);
            let sessions_root = temp.path().join("sessions");
            write_open_turn(&sessions_root, session_id.as_ref(), "go", "partial");
            end_fixture_turn(&sessions_root, session_id.as_ref(), StopReason::Cancelled);
            let (run_locks, completing) = locks();
            let (transitions, _recording) = recording_transitions(&pool);
            begin_human_turn(
                &pool,
                &run_locks,
                &completing,
                &transitions,
                session_id.as_ref(),
            )
            .await
            .unwrap();

            let report = PromptEndReport::new();
            report.record(end);
            park_ended_turn(
                &pool,
                &sessions_root,
                &temp.path().join("baselines"),
                &transitions,
                &node_run_id,
                Some(report),
                crate::workflow::run::completion_snapshot::CompletionPolicy::EndTurn,
                false,
                None,
                None,
            )
            .await
            .unwrap();

            let parked = parked_node(&pool, &run_id, "agent");
            assert_eq!(parked.status, WorkflowNodeStatus::Pending);
            assert_eq!(
                parse_completion_snapshot(parked.payload.as_deref()),
                Some(CompletionSnapshot {
                    user_seq: Some(0),
                    turn_key: Some(TurnKey {
                        user_seq: 0,
                        ended_seq: 2,
                    }),
                    stop_reason: Some("cancelled".to_string()),
                    turn_origin: Some(TurnOrigin::Human),
                    stop_cause: Some(expected),
                    park_kind: Some(ParkKind::Abnormal),
                    blockers: vec![CompletionBlocker {
                        kind: CompletionBlockerKind::TurnStopped,
                        params: Value::Null,
                    }],
                    warnings: Vec::new(),
                    output_digest: Some(format!("sha256:{}", sha256_hex(b"partial"))),
                    provisional: false,
                }),
                "a turn the runtime ended as {end:?} must park as {expected:?}, not just cancelled"
            );
        }
    });
}

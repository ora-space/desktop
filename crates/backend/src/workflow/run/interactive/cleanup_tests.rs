//! A dropped human turn must park its node with the completion snapshot completion later reads.

use super::super::completion_snapshot::{
    CompletionBlocker, CompletionBlockerKind, CompletionSnapshot, ParkKind, StopCause, TurnKey,
    TurnOrigin, parse_completion_snapshot,
};
use super::completion::{CompletionApproval, prepare_completion};
use super::session::{begin_human_turn, end_human_turn, restore_human_turn};
use crate::workflow::run::test_fixture::{
    AGENT_GRAPH, INTERACTIVE_AGENT_GRAPH, bind_and_park, bootstrap, end_fixture_turn, locks,
    recording_transitions, run_test, started_run, write_open_turn,
};
use agent_client_protocol_schema::v1::StopReason;
use ora_application::WorkflowRunRepository;
use ora_db::{RepositoryPool, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId};
use ora_utils::hash::sha256_hex;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::time::Duration;

/// Returns the named node's persisted row so its parked status and snapshot assert together.
fn parked_node(pool: &RepositoryPool, run_id: &WorkflowRunId, node_id: &str) -> WorkflowNodeRun {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
        .into_iter()
        .find(|node| node.node_id == node_id)
        .expect("agent node run")
}

/// A `TurnEnded` appended while cleanup polls still produces the settled snapshot.
///
/// The runtime may confirm the cancelled operation before its history flush lands, so cleanup
/// must not park an unknown snapshot while the turn boundary is already on its way.
#[test]
fn cleanup_polls_until_the_turn_ended_is_durable() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let sessions_root = temp.path().join("sessions");
        write_open_turn(&sessions_root, session_id.as_ref(), "go", "done");
        let (run_locks, completing) = locks();
        let (transitions, recording) = recording_transitions(&pool);
        begin_human_turn(
            &pool,
            &run_locks,
            &completing,
            &transitions,
            session_id.as_ref(),
        )
        .await
        .unwrap();

        let late_root = sessions_root.clone();
        let late_session = session_id.to_string();
        let late_boundary = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            end_fixture_turn(&late_root, &late_session, StopReason::EndTurn);
        });
        end_human_turn(
            &pool,
            &sessions_root,
            &temp.path().join("baselines"),
            &transitions,
            &node_run_id,
            None,
        )
        .await
        .unwrap();
        late_boundary.await.unwrap();

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
                stop_reason: Some("end_turn".to_string()),
                turn_origin: Some(TurnOrigin::Human),
                stop_cause: None,
                park_kind: None,
                blockers: Vec::new(),
                warnings: Vec::new(),
                output_digest: Some(format!("sha256:{}", sha256_hex(b"done"))),
                provisional: false,
            })
        );
        assert_eq!(
            *recording.published.lock().unwrap(),
            vec![run_id.to_string(), run_id.to_string()],
            "beginning and parking each publish one run invalidation"
        );
    });
}

/// A turn that never writes `TurnEnded` parks provisionally instead of stranding the node.
///
/// The provisional snapshot keeps the `userSeq` of the turn it describes and names the reason the
/// turn did not settle, so a later completion can repair it from history by that identity (or the
/// user can explicitly force it) instead of being told the history was unavailable.
#[test]
fn cleanup_parks_provisionally_when_the_turn_never_ends() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let sessions_root = temp.path().join("sessions");
        write_open_turn(&sessions_root, session_id.as_ref(), "go", "still going");
        let (run_locks, completing) = locks();
        let (transitions, recording) = recording_transitions(&pool);
        begin_human_turn(
            &pool,
            &run_locks,
            &completing,
            &transitions,
            session_id.as_ref(),
        )
        .await
        .unwrap();

        end_human_turn(
            &pool,
            &sessions_root,
            &temp.path().join("baselines"),
            &transitions,
            &node_run_id,
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
                turn_key: None,
                stop_reason: None,
                turn_origin: Some(TurnOrigin::Human),
                stop_cause: None,
                park_kind: Some(ParkKind::AwaitingUser),
                blockers: vec![CompletionBlocker {
                    kind: CompletionBlockerKind::TurnUnknown,
                    params: Value::Null,
                }],
                warnings: vec!["turn_not_settled".to_string()],
                output_digest: None,
                provisional: true,
            })
        );
        assert_eq!(
            *recording.published.lock().unwrap(),
            vec![run_id.to_string(), run_id.to_string()],
            "the provisional park still publishes its run invalidation"
        );
    });
}

/// A refused continue prompt restores the parked node and leaves its stopped turn forceable.
///
/// The failure happens before the runtime accepts the prompt, so no user message and no new
/// `TurnEnded` exist. Restoring must put the node back at `Pending` without overwriting the
/// snapshot of the turn it returned to, or the user could neither continue nor force the step.
#[test]
fn a_failed_human_turn_restores_the_previous_snapshot_and_still_forces() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, INTERACTIVE_AGENT_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let sessions_root = temp.path().join("sessions");
        let (run_locks, completing) = locks();
        let (transitions, _recording) = recording_transitions(&pool);

        // The first turn is stopped, so its snapshot parks with the blocker force exists for.
        begin_human_turn(
            &pool,
            &run_locks,
            &completing,
            &transitions,
            session_id.as_ref(),
        )
        .await
        .unwrap();
        write_open_turn(&sessions_root, session_id.as_ref(), "go", "partial");
        end_fixture_turn(&sessions_root, session_id.as_ref(), StopReason::Cancelled);
        end_human_turn(
            &pool,
            &sessions_root,
            &temp.path().join("baselines"),
            &transitions,
            &node_run_id,
            None,
        )
        .await
        .unwrap();
        let stopped =
            parse_completion_snapshot(parked_node(&pool, &run_id, "agent").payload.as_deref())
                .expect("the stopped turn parked a snapshot");

        // Admission flips the node to `Running`; the refused prompt restores the parked state.
        begin_human_turn(
            &pool,
            &run_locks,
            &completing,
            &transitions,
            session_id.as_ref(),
        )
        .await
        .unwrap();
        restore_human_turn(&transitions, &node_run_id)
            .await
            .unwrap();

        let restored = parked_node(&pool, &run_id, "agent");
        assert_eq!(restored.status, WorkflowNodeStatus::Pending);
        assert_eq!(
            parse_completion_snapshot(restored.payload.as_deref()),
            Some(stopped),
            "a restore must not overwrite the snapshot of the turn it returned to"
        );

        // The stopped turn is still completable when the user explicitly forces it.
        let prepared = prepare_completion(
            &pool,
            &sessions_root,
            &temp.path().join("baselines"),
            &temp.path().join("fixture-project"),
            &run_id,
            "agent",
            CompletionApproval::FORCE,
        )
        .expect("the restored stopped turn is still forceable");
        assert_eq!(prepared.output.as_deref(), Some("partial"));
        assert!(
            prepared.payload_patch.is_some(),
            "the forced completion records why it overrode the blocker"
        );
    });
}

/// The user's own stop parks as awaiting participation, not as an abnormal end.
///
/// The stop reason alone cannot tell a user cancel from an inactivity timeout; the recorded cause
/// is what keeps the card from presenting a timeout as the user's own decision (ADR D4).
#[test]
fn a_user_stop_parks_as_awaiting_participation() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, INTERACTIVE_AGENT_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let sessions_root = temp.path().join("sessions");
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
        write_open_turn(&sessions_root, session_id.as_ref(), "go", "partial");
        end_fixture_turn(&sessions_root, session_id.as_ref(), StopReason::Cancelled);

        end_human_turn(
            &pool,
            &sessions_root,
            &temp.path().join("baselines"),
            &transitions,
            &node_run_id,
            Some(StopCause::UserStopped),
        )
        .await
        .unwrap();

        let parked =
            parse_completion_snapshot(parked_node(&pool, &run_id, "agent").payload.as_deref())
                .expect("the stopped turn parked a snapshot");
        assert_eq!(parked.stop_cause, Some(StopCause::UserStopped));
        assert_eq!(parked.park_kind, Some(ParkKind::AwaitingUser));
        assert_eq!(
            parked.blockers,
            vec![CompletionBlocker {
                kind: CompletionBlockerKind::TurnStopped,
                params: Value::Null,
            }]
        );
    });
}

/// A silent timeout, a dropped stream and a crashed session park as abnormal with their own cause.
///
/// All three end as a `Cancelled` turn in history, so the stop reason alone cannot tell them from a
/// stop the user chose. This drives the real cleanup against a real history file and SQLite row: the
/// persisted snapshot — what the read projection and the completion gate read — must name the side
/// that actually ended the turn and classify the park as abnormal (ADR D4).
#[test]
fn silent_timeout_stream_drop_and_session_crash_park_as_abnormal_with_their_cause() {
    run_test(async {
        for cause in [
            StopCause::PromptTimedOut,
            StopCause::StreamDropped,
            StopCause::SessionCrashed,
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

            end_human_turn(
                &pool,
                &sessions_root,
                &temp.path().join("baselines"),
                &transitions,
                &node_run_id,
                Some(cause),
            )
            .await
            .unwrap();

            let parked =
                parse_completion_snapshot(parked_node(&pool, &run_id, "agent").payload.as_deref())
                    .expect("the abnormally ended turn parked a snapshot");
            assert_eq!(
                parked,
                CompletionSnapshot {
                    user_seq: Some(0),
                    turn_key: Some(TurnKey {
                        user_seq: 0,
                        ended_seq: 2,
                    }),
                    stop_reason: Some("cancelled".to_string()),
                    turn_origin: Some(TurnOrigin::Human),
                    stop_cause: Some(cause),
                    park_kind: Some(ParkKind::Abnormal),
                    blockers: vec![CompletionBlocker {
                        kind: CompletionBlockerKind::TurnStopped,
                        params: Value::Null,
                    }],
                    warnings: Vec::new(),
                    output_digest: Some(format!("sha256:{}", sha256_hex(b"partial"))),
                    provisional: false,
                },
                "a turn ended by {cause:?} must park with that cause and as abnormal"
            );
        }
    });
}

/// An interactive graph whose completion requires a changed file, so a missing baseline is visible.
const INTERACTIVE_MUST_CHANGE_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"reports/keep.md","mustChange":true}]}}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

/// The park snapshot carries `file_check_unavailable` when the `mustChange` baseline is absent.
#[test]
fn a_missing_baseline_warns_in_the_park_snapshot() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, INTERACTIVE_MUST_CHANGE_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let sessions_root = temp.path().join("sessions");
        let baselines_root = temp.path().join("baselines");
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
        write_open_turn(&sessions_root, session_id.as_ref(), "go", "done");
        end_fixture_turn(&sessions_root, session_id.as_ref(), StopReason::EndTurn);
        end_human_turn(
            &pool,
            &sessions_root,
            &baselines_root,
            &transitions,
            &node_run_id,
            None,
        )
        .await
        .unwrap();

        let snapshot =
            parse_completion_snapshot(parked_node(&pool, &run_id, "agent").payload.as_deref())
                .expect("the park wrote a completion snapshot");
        assert_eq!(
            snapshot.warnings,
            vec!["file_check_unavailable".to_string()]
        );
        assert_eq!(snapshot.blockers, Vec::new());
    });
}

/// A baseline file that exists but cannot be parsed is unavailable, not silently usable.
#[test]
fn a_corrupt_baseline_warns_in_the_park_snapshot() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, INTERACTIVE_MUST_CHANGE_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let sessions_root = temp.path().join("sessions");
        let baselines_root = temp.path().join("baselines");
        std::fs::create_dir_all(&baselines_root).unwrap();
        std::fs::write(
            baselines_root.join(format!("{}.json", node_run_id)),
            "{not json",
        )
        .unwrap();
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
        write_open_turn(&sessions_root, session_id.as_ref(), "go", "done");
        end_fixture_turn(&sessions_root, session_id.as_ref(), StopReason::EndTurn);
        end_human_turn(
            &pool,
            &sessions_root,
            &baselines_root,
            &transitions,
            &node_run_id,
            None,
        )
        .await
        .unwrap();

        let snapshot =
            parse_completion_snapshot(parked_node(&pool, &run_id, "agent").payload.as_deref())
                .expect("the park wrote a completion snapshot");
        assert_eq!(
            snapshot.warnings,
            vec!["file_check_unavailable".to_string()]
        );
        assert_eq!(snapshot.blockers, Vec::new());
    });
}

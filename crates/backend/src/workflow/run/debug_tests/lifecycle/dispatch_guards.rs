use super::*;
use pretty_assertions::assert_eq;

/// A parked interactive attempt needs no stop acknowledgement, but its old session cannot
/// start another turn until a failed operation releases the fence.
#[test]
fn parked_session_rejects_followup_until_the_fence_is_released() {
    use crate::workflow::run::{interactive::begin_human_turn, test_fixture};
    use ora_contracts::{EmptyErrorParams, PublicError};

    run_test(async {
        let mut graph: serde_json::Value = serde_json::from_str(GRAPH).unwrap();
        graph["nodes"][1]["data"]["agentConfig"]["interactive"] = json!(true);
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        let old = f.row("x");
        let repository = SqliteWorkflowRunEngineRepository::new(f.pool.clone());
        repository
            .transition_node_run_status(
                &old.id,
                WorkflowNodeStatus::Running,
                WorkflowNodeStatus::Pending,
                50,
            )
            .unwrap();
        let preview = f.preview(old.id.as_ref(), /*base_revision*/ 0);
        assert!(preview.impact.stop.is_empty());
        assert!(preview.impact.invalidate.contains(&old.id.to_string()));
        f.store
            .claim(&f.commit_request(&preview, "parked"))
            .unwrap();
        let (locks, completing) = test_fixture::locks();
        let (transitions, _) = test_fixture::recording_transitions(&f.pool);
        let prompt = || begin_human_turn(&f.pool, &locks, &completing, &transitions, "session-1");
        for state in ["stopping", "stop_failed"] {
            if state == "stop_failed" {
                assert!(
                    f.store
                        .record_stop_failure(f.run_id.as_ref(), "parked", "isolation pending", &[])
                        .unwrap()
                );
            }
            assert_eq!(
                prompt().await.unwrap_err().public_error(),
                &PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {}),
                "{state}"
            );
            assert_eq!(f.row("x").status, WorkflowNodeStatus::Pending);
        }
        assert!(
            f.store
                .release_failed_stop(f.run_id.as_ref(), "parked")
                .unwrap()
        );
        prompt().await.unwrap();
        assert_eq!(f.row("x").status, WorkflowNodeStatus::Running);
    });
}

/// The dispatch pointer must end with the run. After a terminal debug restores Failed, an
/// ordinary resume-from-failure must schedule the failed independent node again.
#[test]
fn ordinary_resume_after_restored_debug_is_not_limited_by_old_schedule() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(NoSessions), INDEPENDENT_BRANCH_GRAPH);
        for node in ["x", "s"] {
            let id = f.row(node).id;
            f.engine
                .complete_node(
                    &f.run_id,
                    &id,
                    Some("done".into()),
                    /*structured_output*/ None,
                    /*stop_reason*/ None,
                    vec![],
                )
                .unwrap();
        }
        let independent = f.row("independent").id;
        f.engine
            .fail_node(
                &f.run_id,
                &independent,
                ora_application::NodeFailure::new(
                    ora_application::NodeFailureKind::Session,
                    "independent failure",
                ),
            )
            .unwrap();
        assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
        let preview = f.preview(f.row("x").id.as_ref(), /*base_revision*/ 0);
        assert_eq!(preview.impact.schedule, ["x"]);
        assert_eq!(
            f.coordinator
                .commit(f.commit_request(&preview, "edit-x"))
                .await
                .unwrap()
                .phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
        let new_x = f.row("x").id;
        f.engine
            .complete_node(
                &f.run_id,
                &new_x,
                Some("revised".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
        assert_eq!(dispatched(&f.executor, "independent"), 1);
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(
            (dispatched(&f.executor, "independent"), f.run_status()),
            (2, WorkflowRunStatus::Running)
        );
    });
}

/// A confirmed but undispatched scope is resumed at boot even if an independent interactive
/// node is parked awaiting input.
#[test]
fn boot_dispatches_confirmed_scope_beside_a_parked_interactive_node() {
    run_test(async {
        let graph = r#"{"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"x","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"old"}}},
            {"id":"s","data":{"kind":"agent","agentConfig":{"interactive":true,"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"chat"}}}
        ],"edges":[{"source":"start","target":"x"},{"source":"start","target":"s"}]}"#;
        let f = Fixture::with_graph(Arc::new(NoSessions), graph);
        let s = f.row("s").id;
        rusqlite::Connection::open(f._temp.path().join("repository.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE workflow_node_runs SET status=?1,session_id='session-s' WHERE id=?2",
                rusqlite::params![WorkflowNodeStatus::Pending.database_value(), s.as_ref()],
            )
            .unwrap();
        let req = f.commit_request(
            &f.preview(f.row("x").id.as_ref(), /*base_revision*/ 0),
            "apply",
        );
        f.store.claim(&req).unwrap();
        f.store
            .apply_after_stop(
                f.run_id.as_ref(),
                "apply",
                /*now*/ 60,
                crate::workflow::run::debug::strategy::settle,
            )
            .unwrap();
        assert!(f.store.has_dispatch(f.run_id.as_ref()).unwrap());
        boot_with_calls(&f._temp, "x");
        assert_eq!(f.row("x").status, WorkflowNodeStatus::Running);
        assert_eq!(
            (f.row("s").status, f.run_status()),
            (WorkflowNodeStatus::Pending, WorkflowRunStatus::Running)
        );
    });
}

/// While x's downstream y is stopping, the independent s completes. The join w (x and s) is
/// in the confirmed scope and must wait for the new x instead of dispatching on the old output.
#[test]
fn fenced_join_waits_for_the_new_attempt_while_stopping() {
    run_test(async {
        let agent = |id: &str| {
            json!({"id": id, "data": {"kind": "agent", "agentConfig": {
                "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
                "executor": {"agentCli": "c", "modelId": "m"}, "prompt": id
            }}})
        };
        let graph = json!({
            "nodes": [{"id":"start","data":{"kind":"start"}}, agent("x"), agent("y"), agent("s"), agent("w")],
            "edges": [
                {"source":"start","target":"x"}, {"source":"x","target":"y"}, {"source":"start","target":"s"},
                {"source":"x","target":"w"}, {"source":"s","target":"w"}
            ]
        }).to_string();
        let barrier = Arc::new(StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &graph);
        let old_x = f.row("x").id;
        f.engine
            .complete_node(
                &f.run_id,
                &old_x,
                Some("old".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        let y = f.row("y").id;
        rusqlite::Connection::open(f._temp.path().join("repository.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE workflow_node_runs SET session_id='session-y' WHERE id=?1",
                [y.as_ref()],
            )
            .unwrap();
        let preview = f.preview(old_x.as_ref(), /*base_revision*/ 0);
        assert_eq!(preview.impact.stop, [y.to_string()]);
        assert!(preview.impact.schedule.contains(&"w".to_string()));
        let s = f.row("s").id;
        let (receipt, during) = tokio::join!(
            f.coordinator.commit(f.commit_request(&preview, "edit-x")),
            async {
                barrier.entered.notified().await;
                f.engine
                    .complete_node(
                        &f.run_id,
                        &s,
                        Some("s".into()),
                        /*structured_output*/ None,
                        /*stop_reason*/ None,
                        vec![],
                    )
                    .unwrap();
                let during = dispatched(&f.executor, "w");
                barrier.release.notify_one();
                during
            }
        );
        assert_eq!(
            receipt.unwrap().phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
        assert_eq!(during, 0);
        assert_eq!(dispatched(&f.executor, "w"), 0);
        let new_x = f.row("x").id;
        assert_ne!(new_x, old_x);
        f.engine
            .complete_node(
                &f.run_id,
                &new_x,
                Some("new".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        assert_eq!(
            (dispatched(&f.executor, "w"), dispatched(&f.executor, "y")),
            (1, 2)
        );
    });
}

/// After a restart the unproven old attempt stays quarantined. A late failure or
/// completion from an external writer must not settle it or end the run under the fence.
#[test]
fn late_callbacks_of_a_quarantined_attempt_are_ignored() {
    run_test(async {
        let f = Fixture::new(Arc::new(NoSessions));
        let s = f.row("s").id;
        f.engine
            .complete_node(
                &f.run_id,
                &s,
                Some("independent".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        let old = f.row("x");
        let req = f.commit_request(&f.preview(old.id.as_ref(), /*base_revision*/ 0), "restart");
        f.store.claim(&req).unwrap();
        boot_with_calls(&f._temp, "");
        assert!(matches!(
            f.coordinator.commit(req).await.unwrap().phase,
            WorkflowDebugPhase::StopFailed { .. }
        ));
        f.engine
            .fail_node(
                &f.run_id,
                &old.id,
                ora_application::NodeFailure::new(
                    ora_application::NodeFailureKind::Session,
                    "late failure",
                ),
            )
            .unwrap();
        assert_eq!(
            (f.row("x").id, f.row("x").status, f.run_status()),
            (
                old.id.clone(),
                WorkflowNodeStatus::Running,
                WorkflowRunStatus::Running
            )
        );
        f.engine
            .complete_node(
                &f.run_id,
                &old.id,
                Some("late".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        assert_eq!(
            (f.row("x").status, f.row("x").output),
            (WorkflowNodeStatus::Running, None)
        );
    });
}
/// An interrupted independent attempt fails the run at boot (FailRun), but the
/// quarantined old x stays untouched and is still named by the failed-stop receipt.
#[test]
fn boot_failrun_keeps_the_quarantined_attempt() {
    run_test(async {
        let f = Fixture::new(Arc::new(NoSessions));
        let old = f.row("x");
        let req = f.commit_request(&f.preview(old.id.as_ref(), /*base_revision*/ 0), "restart");
        f.store.claim(&req).unwrap();
        boot_with_calls(&f._temp, "");
        assert_eq!(
            (f.row("s").status, f.run_status()),
            (WorkflowNodeStatus::Failed, WorkflowRunStatus::Failed)
        );
        assert_eq!(
            (f.row("x").id, f.row("x").status, f.row("x").error),
            (old.id.clone(), WorkflowNodeStatus::Running, None)
        );
        let phase = f.coordinator.commit(req).await.unwrap().phase;
        assert!(
            matches!(&phase, WorkflowDebugPhase::StopFailed { unsettled_node_run_ids, .. } if unsettled_node_run_ids == &[old.id.to_string()]),
            "{phase:?}"
        );
    });
}

//! Where the debug exemptions at boot and the parked round reopen stop applying (review 1: M2,
//! L1, L2, L3). Outside "reopened and not dispatched since", a Loop is ordinary interrupted work
//! (while D6); a parked reopen is performed at most once and only while it still matches its Loop.
use super::*;
use ora_domain::WorkflowScopeStatus;
use pretty_assertions::assert_eq;

/// One scope's persisted status.
fn scope_status(f: &Fixture, scope_id: &str) -> i64 {
    let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
    db.query_row(
        "SELECT status FROM workflow_execution_scopes WHERE id = ?1",
        [scope_id],
        |row| row.get(0),
    )
    .unwrap()
}

/// Runs the production boot with a fresh engine and executor, as a new process would.
fn boot(f: &Fixture) -> RecordingExecutor {
    let executor = RecordingExecutor::default();
    let engine = Arc::new(WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(f.pool.clone()),
        executor.clone(),
        UuidWorkflowNodeRunIdGenerator::new(),
        SystemClock,
    ));
    super::super::super::recovery::run_workflow_run_boot_sweep(
        &f.pool,
        &engine,
        &KeyedResourceLocks::new(),
        SystemClock,
    );
    executor
}

/// The Loop row's status and error, plus whether the run failed.
fn loop_outcome(f: &Fixture) -> (WorkflowRunStatus, WorkflowNodeStatus, Option<String>) {
    let owner = f.row("loop");
    (f.run_status(), owner.status, owner.error)
}

const INTERRUPTED: &str = r#"{"reason":"interrupted_by_restart"}"#;

/// The owner's parked reopens, if any.
fn parked_on_owner(f: &Fixture) -> Option<serde_json::Value> {
    let owner: serde_json::Value =
        serde_json::from_str(f.row("loop").payload.as_deref().unwrap_or("{}")).unwrap();
    owner.get("debugWhileReplays").cloned()
}

/// Complete the live `outside` attempt.
fn finish_outside(f: &Fixture) {
    f.engine
        .complete_node(
            &f.run_id,
            &f.row("outside").id,
            Some("recovered".into()),
            None,
            None,
            vec![],
        )
        .unwrap();
}

/// Fail the live `outside` attempt, failing the run.
fn fail_outside(f: &Fixture) {
    f.engine
        .fail_node(
            &f.run_id,
            &f.row("outside").id,
            ora_application::NodeFailure::new(
                ora_application::NodeFailureKind::Session,
                "outside failed again",
            ),
        )
        .unwrap();
    assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
}

/// Core case "applied, then the new revision's while attempt is claimed and the process stops
/// again": ordinary boot cleanup settles it per while D6 and the coordinator does not dispatch
/// the position a second time (D5.5).
#[test]
fn a_claimed_replacement_interrupted_by_a_second_crash_settles_per_while_d6() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
        let selected = f.observe_live_writer();
        f.finish_writer("done");
        let preview = f.prompt_preview(&selected.node_run_id, 0, "new");
        f.store.claim(&f.commit_request(&preview, "crash")).unwrap();
        let first = boot(&f);
        let replacement = last_dispatch(&first, "writer");
        let writer = |f: &Fixture| {
            node_runs(&f.pool, &f.run_id)
                .into_iter()
                .find(|row| row.id.as_ref() == replacement.node_run_id)
                .unwrap()
        };
        assert_eq!(writer(&f).status, WorkflowNodeStatus::Running);

        let second = boot(&f);
        let claimed = writer(&f);
        assert_eq!(
            (
                loop_outcome(&f),
                claimed.status,
                claimed
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("interrupted_by_restart")),
                scope_status(&f, claimed.scope_id.as_ref()),
                second.records().len(),
            ),
            (
                (
                    WorkflowRunStatus::Failed,
                    WorkflowNodeStatus::Failed,
                    Some(INTERRUPTED.to_string())
                ),
                WorkflowNodeStatus::Failed,
                true,
                WorkflowScopeStatus::Failed.database_value(),
                0
            )
        );
    });
}

/// A round reopened by an applied edit that still has a kept sibling in flight is interrupted
/// work, not a round awaiting its first dispatch: the restart fails it per while D6.
#[test]
fn a_reopened_round_with_a_running_kept_sibling_still_fails_at_restart() {
    run_test(async {
        let mut graph: serde_json::Value = serde_json::from_str(LOOP_GRAPH).unwrap();
        let mut slow = graph["nodes"][3].clone();
        slow["id"] = json!("slow");
        graph["nodes"].as_array_mut().unwrap().push(slow);
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"entry", "target":"slow"}));
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        let selected = f.observe_live_writer();
        f.finish_writer("done");
        let preview = f.prompt_preview(&selected.node_run_id, 0, "new");
        assert!(preview.impact.stop.is_empty());
        assert!(preview.impact.keep.contains(&f.row("slow").id.to_string()));
        f.store.claim(&f.commit_request(&preview, "kept")).unwrap();
        f.store
            .apply_after_stop(f.run_id.as_ref(), "kept", 60, |tx, p| {
                super::super::super::debug::strategy::settle(tx, f.run_id.as_ref(), 60, p)
            })
            .unwrap();
        assert_eq!(f.row("loop").status, WorkflowNodeStatus::Running);
        let executor = boot(&f);
        assert_eq!(
            (
                loop_outcome(&f),
                scope_status(&f, &f.scope_of(&selected.node_run_id)),
                executor.records().len()
            ),
            (
                (
                    WorkflowRunStatus::Failed,
                    WorkflowNodeStatus::Failed,
                    Some(INTERRUPTED.to_string())
                ),
                WorkflowScopeStatus::Failed.database_value(),
                0
            )
        );
    });
}

/// L1: once the replayed Loop dispatched again, an idle window later in the same execution is an
/// ordinary interrupted Loop, exactly like `an_ordinary_idle_loop_still_fails_at_restart`.
#[test]
fn an_idle_window_after_the_replay_dispatched_fails_at_restart_like_an_ordinary_loop() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        f.edit_round_three_while_outside_fails(&barrier).await;
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        f.finish_writer("T3");
        let fourth = f.observe_live_writer();
        finish_outside(&f);
        // The process stopped between round 4's member settlement and the Loop's next step.
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        db.execute(
            "UPDATE workflow_node_runs SET status = 2 WHERE id = ?1",
            [&fourth.node_run_id],
        )
        .unwrap();
        let executor = boot(&f);
        assert_eq!(
            (
                loop_outcome(&f),
                scope_status(&f, &f.scope_of(&fourth.node_run_id)),
                executor.records().len()
            ),
            (
                (
                    WorkflowRunStatus::Failed,
                    WorkflowNodeStatus::Failed,
                    Some(INTERRUPTED.to_string())
                ),
                WorkflowScopeStatus::Failed.database_value(),
                0
            )
        );
    });
}

/// L2: recovery consumes the parked records. A later failure and recovery of the same run does
/// not replay the edited round again over rounds that already completed.
#[test]
fn parked_records_are_consumed_once_across_repeated_failures() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        f.edit_round_three_while_outside_fails(&barrier).await;
        assert!(parked_on_owner(&f).is_some());
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(parked_on_owner(&f), None);
        f.finish_writer("T3");
        f.finish_writer("done");
        assert_eq!(f.row("loop").status, WorkflowNodeStatus::Succeeded);
        let writers = dispatched(&f.executor, "writer");
        assert_eq!(writers, 6);

        fail_outside(&f);
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(
            (dispatched(&f.executor, "writer"), f.row("loop").status),
            (writers, WorkflowNodeStatus::Succeeded)
        );
        finish_outside(&f);
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(f.row("out").output.as_deref(), Some(r#"{"draft":"done"}"#));
    });
}

/// L3: a parked record that no longer matches its Loop, here left behind while a build that
/// ignores it let the Loop run on to round 5, is dropped instead of rewinding the finished Loop.
#[test]
fn a_stale_parked_round_reopen_is_dropped_instead_of_rewinding_the_loop() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        f.edit_round_three_while_outside_fails(&barrier).await;
        let owner = f.row("loop").id;
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        let record: String = db
            .query_row(
                "SELECT json_extract(payload, '$.debugWhileReplays') FROM workflow_node_runs WHERE id = ?1",
                [owner.as_ref()],
                |row| row.get(0),
            )
            .unwrap();
        // The other build recovers without the record, then the Loop runs on and finishes.
        db.execute(
            "UPDATE workflow_node_runs SET payload = json_remove(payload, '$.debugWhileReplays') WHERE id = ?1",
            [owner.as_ref()],
        )
        .unwrap();
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        f.finish_writer("R4");
        f.finish_writer("done");
        assert_eq!(f.row("loop").status, WorkflowNodeStatus::Succeeded);
        db.execute(
            "UPDATE workflow_node_runs SET payload = json_set(payload, '$.debugWhileReplays', json(?2)) WHERE id = ?1",
            (owner.as_ref(), &record),
        )
        .unwrap();
        let settled = f.row("loop");
        let writers = dispatched(&f.executor, "writer");

        fail_outside(&f);
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(parked_on_owner(&f), None);
        assert_eq!(
            (dispatched(&f.executor, "writer"), f.row("loop").status),
            (writers, settled.status)
        );
        finish_outside(&f);
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(f.row("out").output.as_deref(), Some(r#"{"draft":"done"}"#));
    });
}

/// Five-round Loop with `outside` and, when `post` is set, a downstream agent `post` reading
/// the Loop's export. Rounds before `live` complete with S1, S2, …; returns the round-3 and
/// round-4 writers. Then edit round 3 while `outside` fails during the stop of `stop_node`'s
/// live attempt (the reverse race at a larger distance than round 4, review 2 R2-H1).
async fn reverse_race_at_round_three(
    f: &Fixture,
    barrier: &super::super::lifecycle::StopBarrier,
    finished: &[&str],
    stop_node: &str,
) -> (DispatchRecord, DispatchRecord) {
    let mut observed = Vec::new();
    for value in finished {
        observed.push(f.observe_live_writer());
        f.finish_writer(value);
    }
    let live = last_dispatch(&f.executor, stop_node);
    f.bind_agent(&WorkflowNodeRunId::new(&live.node_run_id));
    let preview = f.prompt_preview(&observed[2].node_run_id, 0, "new");
    assert_eq!(
        (preview.target.round, preview.impact.stop.clone()),
        (Some(3), vec![live.node_run_id.clone()])
    );
    let (op, ()) = tokio::join!(
        f.coordinator.commit(f.commit_request(&preview, "race")),
        async {
            barrier.entered.notified().await;
            fail_outside(f);
            barrier.release.notify_one();
        }
    );
    assert_eq!(
        op.unwrap().phase,
        WorkflowDebugPhase::Applied { revision: 1 }
    );
    assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
    (observed[2].clone(), observed[3].clone())
}

/// Scenario J: round 3 is edited while round 5 runs. Ordinary recovery reruns round 3 in its
/// own scope with the new prompt and its entrance value S2, not round 5 with the retired S4.
#[test]
fn ordinary_recovery_replays_an_edited_round_two_rounds_behind_the_live_one() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        let (round_three, round_four) =
            reverse_race_at_round_three(&f, &barrier, &["S1", "S2", "S3", "S4"], "writer").await;
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(parked_on_owner(&f), None);
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&replay.node_run_id),
                replay.variable_pool.values.contains_key("writer.output"),
            ),
            (
                Some("new"),
                Some("S2"),
                f.scope_of(&round_three.node_run_id),
                false
            )
        );
        f.finish_writer("T3");
        let next = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                next.prompt.as_deref(),
                next.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&next.node_run_id),
            ),
            (Some("new"), Some("T3"), f.scope_of(&round_four.node_run_id))
        );
    });
}

/// Scenario K: the Loop already finished all five rounds and its consumer `post` is in flight
/// when round 3 is edited. Recovery withdraws the Loop's export and the output node's value
/// before dispatching, and reruns round 3 with the new prompt and S2.
#[test]
fn ordinary_recovery_replays_an_edited_round_of_a_finished_loop() {
    run_test(async {
        let mut graph: serde_json::Value =
            serde_json::from_str(&five_rounds_with_outside()).unwrap();
        graph["nodes"].as_array_mut().unwrap().push(json!({
            "id":"post", "data":{"kind":"agent", "agentConfig":{
                "retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},
                "executor":{"agentCli":"c","modelId":"m"}, "prompt":"post {{#loop.result#}}"
            }}
        }));
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"loop","target":"post"}));
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &graph.to_string());
        let (round_three, _) =
            reverse_race_at_round_three(&f, &barrier, &["S1", "S2", "S3", "S4", "done"], "post")
                .await;
        assert_eq!(f.row("loop").status, WorkflowNodeStatus::Succeeded);
        let stale = ["loop.result", "out.output"];
        let present = |f: &Fixture| {
            let run = SqliteWorkflowRunRepository::new(f.pool.clone())
                .find_run(&f.run_id)
                .unwrap()
                .unwrap();
            let payload: ora_application::WorkflowRunPayload =
                serde_json::from_str(run.payload.as_deref().unwrap()).unwrap();
            stale
                .iter()
                .filter(|key| payload.variable_pool.values.contains_key(**key))
                .map(|key| key.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(present(&f), stale.map(String::from));
        let skip = f.executor.records().len();
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(present(&f), Vec::<String>::new());
        let redispatched: Vec<_> = f.executor.records().into_iter().skip(skip).collect();
        let mut nodes: Vec<_> = redispatched.iter().map(|r| r.node_id.clone()).collect();
        nodes.sort();
        assert_eq!(nodes, ["outside", "writer"]);
        assert!(
            redispatched
                .iter()
                .all(|r| !r.variable_pool.values.contains_key("loop.result"))
        );
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&replay.node_run_id),
            ),
            (
                Some("new"),
                Some("S2"),
                f.scope_of(&round_three.node_run_id)
            )
        );
        f.finish_writer("done");
        assert_eq!(
            (
                dispatched(&f.executor, "post"),
                f.row("loop").output.as_deref()
            ),
            (2, Some(r#"{"result":"done"}"#))
        );
    });
}

/// Review 2 R2-L1: a second edit on an already debugged Loop is anchored on its own revision.
/// After a first edit at round 1 reran to completion, a second edit at round 3 that crashes
/// after acceptance still reopens and dispatches round 3 at boot.
#[test]
fn a_second_edit_of_a_debugged_loop_dispatches_at_boot() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
        let first = f.observe_live_writer();
        f.finish_writer("done");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        let op = f
            .coordinator
            .commit(f.commit_request(&f.prompt_preview(&first.node_run_id, 0, "new1"), "first"))
            .await
            .unwrap();
        assert_eq!(op.phase, WorkflowDebugPhase::Applied { revision: 1 });
        f.finish_writer("T1");
        f.finish_writer("T2");
        let third = f.observe_live_writer();
        f.finish_writer("done");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        let preview = f.prompt_preview(&third.node_run_id, 1, "new3");
        assert_eq!(
            (preview.target.round, preview.impact.stop.is_empty()),
            (Some(3), true)
        );
        f.store
            .claim(&f.commit_request(&preview, "second"))
            .unwrap();
        let executor = boot(&f);
        let replay = last_dispatch(&executor, "writer");
        assert_eq!(
            (
                f.run_status(),
                executor.records().len(),
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
            ),
            (WorkflowRunStatus::Running, 1, Some("new3"), Some("T2"))
        );
    });
}

/// Review 2 R2-L2: only an applied edit reopens a container. A Loop pointed at by a released
/// failed stop alone is ordinary interrupted work at restart (while D6).
#[test]
fn a_loop_named_only_by_a_released_failed_stop_still_fails_at_restart() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(UnprovableStop), LOOP_GRAPH);
        let selected = f.observe_live_writer();
        f.finish_writer("again");
        let second = f.observe_live_writer();
        f.bind_agent(&WorkflowNodeRunId::new(&second.node_run_id));
        let preview = f.prompt_preview(&selected.node_run_id, 0, "new");
        assert_eq!(preview.impact.stop, vec![second.node_run_id.clone()]);
        let receipt = f
            .coordinator
            .commit(f.commit_request(&preview, "unproven"))
            .await
            .unwrap();
        assert!(matches!(
            receipt.phase,
            WorkflowDebugPhase::StopFailed { .. }
        ));
        assert!(
            f.store
                .release_failed_stop(f.run_id.as_ref(), "unproven")
                .unwrap()
        );
        // The stopped writer was cancelled and nothing else runs in the round: an idle window.
        assert_eq!(
            (f.run_status(), f.row("loop").status),
            (WorkflowRunStatus::Running, WorkflowNodeStatus::Running)
        );
        let executor = boot(&f);
        assert_eq!(
            (
                loop_outcome(&f),
                scope_status(&f, &f.scope_of(&second.node_run_id)),
                executor.records().len()
            ),
            (
                (
                    WorkflowRunStatus::Failed,
                    WorkflowNodeStatus::Failed,
                    Some(INTERRUPTED.to_string())
                ),
                WorkflowScopeStatus::Failed.database_value(),
                0
            )
        );
    });
}

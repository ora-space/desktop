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

//! The production retry timer over real SQLite, the system clock, and the tokio runtime.
//!
//! Waits here are real (one second at most), so these tests prove the backend wiring: the timer
//! sleeps until the persisted deadline, wakes the engine under the run lock on the blocking
//! pool, and a wake that outlived its wait — after cancel or a restart — starts nothing.

use super::engine::ConcreteWorkflowRunEngine;
use super::recovery::run_workflow_run_boot_sweep;
use super::retry_tests::{DispatchLog, Harness, linear, retry, siblings};
use super::retry_timer::WorkflowRetryTimers;
use super::test_fixture::{bootstrap, locks, run_test, seeded_pending_run};
use crate::clock::SystemClock;
use ora_application::{
    CancelWorkflowRunResult, Clock, NodeFailure, NodeFailureKind, UuidWorkflowNodeRunIdGenerator,
    WorkflowRunEngine, WorkflowRunRepository,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Builds the backend's engine composition: the production engine with the tokio retry timer.
fn engine_with_timer(
    pool: &RepositoryPool,
    dispatches: &DispatchLog,
) -> Arc<ConcreteWorkflowRunEngine> {
    let timers = Arc::new(WorkflowRetryTimers::new(locks().0, SystemClock));
    let engine = Arc::new(
        WorkflowRunEngine::new(
            SqliteWorkflowRunEngineRepository::new(pool.clone()),
            dispatches.clone(),
            UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        )
        .with_retry_timer(timers.clone()),
    );
    timers.set_engine(&engine);
    engine
}

fn dispatched(dispatches: &DispatchLog, node_id: &str) -> usize {
    dispatches
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|dispatch| dispatch.node_id == node_id)
        .count()
}

fn live_rows(pool: &RepositoryPool, run_id: &WorkflowRunId, node_id: &str) -> Vec<WorkflowNodeRun> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
        .into_iter()
        .filter(|row| row.node_id == node_id)
        .collect()
}

fn run_status(pool: &RepositoryPool, run_id: &WorkflowRunId) -> WorkflowRunStatus {
    SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .unwrap()
        .unwrap()
        .status
}

fn fail(engine: &ConcreteWorkflowRunEngine, run_id: &WorkflowRunId, node_run: &WorkflowNodeRun) {
    engine
        .fail_node(
            run_id,
            &node_run.id,
            NodeFailure::new(NodeFailureKind::Session, "session dropped"),
        )
        .unwrap();
}

/// Yields to the runtime until `done` holds, for at most five seconds.
async fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "condition not reached in time");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Test 15: each armed wait sleeps until its own persisted deadline — a zero-second retry
/// starts at once, a one-second retry a second later — and the retried run then succeeds.
#[test]
fn the_real_timer_starts_each_retry_at_its_own_deadline() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(
            &temp,
            &pool,
            &siblings(retry(true, 1, 1), retry(true, 1, 0)),
        );
        let dispatches = DispatchLog::default();
        let engine = engine_with_timer(&pool, &dispatches);
        engine.start(&run_id).unwrap();
        let failed_at = Instant::now();
        fail(&engine, &run_id, &live_rows(&pool, &run_id, "a")[0]);
        fail(&engine, &run_id, &live_rows(&pool, &run_id, "b")[0]);
        let a_wait = Harness::wait_of(&live_rows(&pool, &run_id, "a")[0]).unwrap();

        wait_until(|| dispatched(&dispatches, "b") == 2).await;
        assert_eq!(dispatched(&dispatches, "a"), 1);
        wait_until(|| dispatched(&dispatches, "a") == 2).await;
        assert!(
            failed_at.elapsed() >= Duration::from_millis(990),
            "the one-second retry started after {:?}",
            failed_at.elapsed()
        );
        let started = live_rows(&pool, &run_id, "a").pop().unwrap();
        assert!(started.started_at.unwrap() >= a_wait.due_at);
        assert_eq!(Harness::wait_of(&started), None);

        for node_id in ["a", "b"] {
            let row = live_rows(&pool, &run_id, node_id).pop().unwrap();
            engine
                .complete_node(
                    &run_id,
                    &row.id,
                    Some("ok".to_string()),
                    None,
                    None,
                    Vec::new(),
                )
                .unwrap();
        }
        assert_eq!(run_status(&pool, &run_id), WorkflowRunStatus::Succeeded);
    });
}

/// Test 15: cancelling during a real wait settles the waiting attempt at once, and the timer
/// that fires afterwards starts nothing.
#[test]
fn cancel_during_a_real_wait_leaves_nothing_for_the_timer_to_start() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, &linear(retry(true, 2, 1)));
        let dispatches = DispatchLog::default();
        let engine = engine_with_timer(&pool, &dispatches);
        engine.start(&run_id).unwrap();
        fail(&engine, &run_id, &live_rows(&pool, &run_id, "a")[0]);
        let due_at = Harness::wait_of(&live_rows(&pool, &run_id, "a")[0])
            .unwrap()
            .due_at;
        assert_eq!(
            engine.cancel(&run_id).unwrap(),
            CancelWorkflowRunResult::Cancelled
        );

        wait_until(|| SystemClock.now_timestamp_millis() > due_at + 200).await;
        assert_eq!(dispatched(&dispatches, "a"), 1);
        let rows = live_rows(&pool, &run_id, "a");
        assert_eq!(
            (rows.len(), rows[0].status, rows[0].started_at),
            (1, WorkflowNodeStatus::Cancelled, None)
        );
        assert_eq!(run_status(&pool, &run_id), WorkflowRunStatus::Cancelled);
    });
}

/// Test 15 and 9: the real boot sweep of a restarted process interrupts a waiting outer attempt
/// and fails the run; the previous process's timer, still armed, starts nothing.
#[test]
fn a_restart_during_a_real_wait_fails_the_run_and_the_old_timer_starts_nothing() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, &linear(retry(true, 2, 1)));
        let dispatches = DispatchLog::default();
        let engine = engine_with_timer(&pool, &dispatches);
        engine.start(&run_id).unwrap();
        fail(&engine, &run_id, &live_rows(&pool, &run_id, "a")[0]);
        let waiting = live_rows(&pool, &run_id, "a").pop().unwrap();
        let due_at = Harness::wait_of(&waiting).unwrap().due_at;

        let restarted = engine_with_timer(&pool, &dispatches);
        run_workflow_run_boot_sweep(&pool, &restarted, &locks().0, SystemClock);
        assert_eq!(run_status(&pool, &run_id), WorkflowRunStatus::Failed);
        let interrupted = live_rows(&pool, &run_id, "a").pop().unwrap();
        assert_eq!(
            (
                &interrupted.id,
                interrupted.status,
                Harness::wait_of(&interrupted)
            ),
            (&waiting.id, WorkflowNodeStatus::Failed, None)
        );
        assert_eq!(
            Harness::error_detail(&interrupted)["kind"],
            json!("interrupted_by_restart")
        );

        wait_until(|| SystemClock.now_timestamp_millis() > due_at + 200).await;
        assert_eq!(dispatched(&dispatches, "a"), 1);
        assert_eq!(live_rows(&pool, &run_id, "a").pop().unwrap(), interrupted);
    });
}

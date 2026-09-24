//! What a restart during the wait leaves behind in Loops and runs with several waits, and the
//! failed-attempt history `get_workflow_run` returns.

use super::recovery::sweep_one_run;
use super::retry_composite_tests::loop_graph;
use super::retry_tests::{Harness, agent, graph, retry, siblings};
use ora_application::{
    CancelWorkflowRunResult, GetWorkflowRunHandler, NodeAutoRetry, NodeFailure, NodeFailureKind,
    ResumeWorkflowRunResult, WorkflowRunEngineRepository, WorkflowRunRepository,
};
use ora_contracts::{GetWorkflowRunRequest, WorkflowNodeFailedAttempt};
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::sync::Arc;

/// `(status, started_at, still waiting, error_detail.kind)` of a row after the boot sweep.
fn swept(row: &WorkflowNodeRun) -> (WorkflowNodeStatus, Option<i64>, bool, Value) {
    (
        row.status,
        row.started_at,
        Harness::wait_of(row).is_some(),
        Harness::error_detail(row)["kind"].clone(),
    )
}

/// A restart during a Loop body wait: the Loop is not an Iteration region, so the sweep fails the
/// run with the waiting writer and the Loop row. The old timer starts nothing, and a resume
/// reruns the Loop from round 1 with a fresh budget and fresh attempt numbers.
#[test]
fn a_restart_during_a_loop_body_wait_fails_the_run_and_resume_restarts_the_loop() {
    let h = Harness::start(&loop_graph(json!({}), /*sibling*/ None));
    let first = h.running("writer");
    h.fail(&first.id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("writer");
    let old_loop = h.rows_of("loop").pop().unwrap();

    sweep_one_run(&h.repository(), &h.run_id, 5_000).unwrap();
    let writer = h.rows_of("writer").pop().unwrap();
    let loop_row = h.rows_of("loop").pop().unwrap();
    assert_eq!(
        (
            writer.id.clone(),
            swept(&writer),
            loop_row.id.clone(),
            loop_row.status,
            h.run().status
        ),
        (
            waiting.id.clone(),
            (
                WorkflowNodeStatus::Failed,
                None,
                false,
                json!("interrupted_by_restart")
            ),
            old_loop.id.clone(),
            WorkflowNodeStatus::Failed,
            WorkflowRunStatus::Failed
        )
    );

    h.wake(&waiting.id, 11_000);
    assert_eq!(h.dispatched("writer"), vec![first.id.to_string()]);

    h.set_now(20_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let rerun = h.running("writer");
    assert_ne!(rerun.scope_id, waiting.scope_id);
    assert_eq!(
        (
            rerun.started_at,
            NodeAutoRetry::from_payload(rerun.payload.as_deref())
        ),
        (Some(20_000), None)
    );
    h.fail(&rerun.id, NodeFailureKind::Session, 21_000);
    let wait = Harness::wait_of(&h.running("writer")).unwrap();
    assert_eq!((wait.attempt, wait.retry, wait.due_at), (2, 1, 31_000));
}

/// A restart with two nodes waiting fails both waits with the run; neither old timer starts
/// anything, and a resume reruns both nodes at once with fresh budgets.
#[test]
fn a_restart_with_two_waits_fails_both_and_resume_reruns_both() {
    let h = Harness::start(&siblings(json!({}), retry(true, 1, 30)));
    let first_a = h.running("a");
    let first_b = h.running("b");
    h.fail(&first_a.id, NodeFailureKind::Session, 1_000);
    h.fail(&first_b.id, NodeFailureKind::StructuredOutput, 2_000);
    let (waiting_a, waiting_b) = (h.running("a"), h.running("b"));

    sweep_one_run(&h.repository(), &h.run_id, 5_000).unwrap();
    let interrupted = (json!("interrupted_by_restart"), json!(true));
    let detail = |row: &WorkflowNodeRun| {
        let detail = Harness::error_detail(row);
        (detail["kind"].clone(), detail["resumable"].clone())
    };
    let (a, b) = (h.rows_of("a").pop().unwrap(), h.rows_of("b").pop().unwrap());
    assert_eq!(
        (
            (a.id.clone(), a.status, Harness::wait_of(&a), detail(&a)),
            (b.id.clone(), b.status, Harness::wait_of(&b), detail(&b)),
            h.run().status
        ),
        (
            (
                waiting_a.id.clone(),
                WorkflowNodeStatus::Failed,
                None,
                interrupted.clone()
            ),
            (
                waiting_b.id.clone(),
                WorkflowNodeStatus::Failed,
                None,
                interrupted
            ),
            WorkflowRunStatus::Failed
        )
    );

    h.wake(&waiting_a.id, 11_000);
    h.wake(&waiting_b.id, 32_000);
    assert_eq!(
        (h.dispatched("a"), h.dispatched("b")),
        (vec![first_a.id.to_string()], vec![first_b.id.to_string()])
    );

    h.set_now(40_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let (a, b) = (h.running("a"), h.running("b"));
    assert_eq!(
        (
            a.started_at,
            NodeAutoRetry::from_payload(a.payload.as_deref()),
            b.started_at,
            NodeAutoRetry::from_payload(b.payload.as_deref()),
            h.dispatched("a").len(),
            h.dispatched("b").len()
        ),
        (Some(40_000), None, Some(40_000), None, 2, 2)
    );
}

/// `start → a → b`, so each node's rows are created at distinct times.
fn chain(a: Value, b: Value) -> String {
    graph(
        vec![
            json!({"id": "start", "data": {"kind": "start"}}),
            agent("a", a),
            agent("b", b),
        ],
        &[("start", "a"), ("a", "b")],
    )
}

fn failed_attempts(h: &Harness, run_id: &WorkflowRunId) -> Vec<WorkflowNodeFailedAttempt> {
    GetWorkflowRunHandler::new(Arc::new(SqliteWorkflowRunRepository::new(h.pool.clone())))
        .handle(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })
        .unwrap()
        .failed_attempts
        .unwrap()
}

fn expected_attempt(
    row: &WorkflowNodeRun,
    attempt: u32,
    kind: &str,
    message: &str,
    source_chain: &[&str],
    recorded_at: i64,
) -> WorkflowNodeFailedAttempt {
    WorkflowNodeFailedAttempt {
        node_run_id: row.id.to_string(),
        node_id: row.node_id.clone(),
        scope_id: row.scope_id.to_string(),
        iteration: None,
        session_id: None,
        attempt,
        kind: kind.to_string(),
        message: message.to_string(),
        source_chain: source_chain.iter().map(ToString::to_string).collect(),
        recorded_at,
        started_at: row.started_at,
        finished_at: Some(recorded_at),
    }
}

/// The history lists the run's failed attempts oldest first with their full records: one an
/// automatic retry replaced and two that manual resumes replaced. Live rows, a cancelled wait a
/// resume cleared, and every row of another run stay out.
#[test]
fn the_failed_attempt_history_lists_retried_and_resumed_attempts_of_one_run_oldest_first() {
    let h = Harness::pending(&chain(retry(true, 1, 0), retry(false, 0, 0)));
    let mut other = h.run();
    other.id = WorkflowRunId::new("run-2");
    let other_id = other.id.clone();
    SqliteWorkflowRunRepository::new(h.pool.clone())
        .create_run(other)
        .unwrap();
    h.engine.start(&h.run_id).unwrap();

    // `a` fails with a behaviour failure, and its automatic retry succeeds.
    let a1 = h.running("a");
    h.fail_with(
        &a1.id,
        NodeFailure::new(NodeFailureKind::StructuredOutput, "a reply is not JSON")
            .with_source_chain(vec!["expected value at line 1 column 1".to_string()]),
        1_000,
    );
    let a2 = h.running("a");
    h.wake(&a2.id, 2_000);
    h.complete(&a2.id, "a done", 3_000);
    // `b` never retries: it fails, is resumed, fails again, and is resumed into a success.
    let b1 = h.running("b");
    h.fail(&b1.id, NodeFailureKind::Session, 4_000);
    h.set_now(5_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let b2 = h.running("b");
    h.fail(&b2.id, NodeFailureKind::Session, 6_000);
    h.set_now(7_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    h.complete(&h.running("b").id, "b done", 8_000);
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);

    // The other run fails `a` once, is cancelled during the wait, and is resumed.
    h.set_now(9_000);
    h.engine.start(&other_id).unwrap();
    let engine_rows = SqliteWorkflowRunEngineRepository::new(h.pool.clone());
    let running_a = |run_id: &WorkflowRunId| {
        engine_rows
            .list_node_runs(run_id)
            .unwrap()
            .into_iter()
            .find(|row| row.node_id == "a" && row.status == WorkflowNodeStatus::Running)
            .unwrap()
    };
    let other_a = running_a(&other_id);
    h.set_now(10_000);
    h.engine
        .fail_node(
            &other_id,
            &other_a.id,
            NodeFailure::new(NodeFailureKind::Session, "other run failed"),
        )
        .unwrap();
    h.set_now(10_500);
    assert_eq!(
        h.engine.cancel(&other_id).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    h.set_now(11_000);
    assert_eq!(
        h.engine.resume_from_failure(&other_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    assert_eq!(running_a(&other_id).started_at, Some(11_000));

    assert_eq!(
        failed_attempts(&h, &h.run_id),
        vec![
            expected_attempt(
                &a1,
                1,
                "structured_output",
                "a reply is not JSON",
                &["expected value at line 1 column 1"],
                1_000
            ),
            expected_attempt(&b1, 1, "session", "Session at 4000", &[], 4_000),
            expected_attempt(&b2, 2, "session", "Session at 6000", &[], 6_000),
        ]
    );
    assert_eq!(
        failed_attempts(&h, &other_id),
        vec![expected_attempt(
            &other_a,
            1,
            "session",
            "other run failed",
            &[],
            10_000
        )]
    );
}

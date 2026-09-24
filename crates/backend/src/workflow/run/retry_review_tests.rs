//! Regression tests for automatic-retry edge cases: which earlier failure a new
//! attempt is told about, and what happens when a wake or a retry decision hits an error.

use super::recovery::sweep_one_run;
use super::retry_tests::{Harness, agent, graph, linear, retry};
use ora_application::{
    NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, WorkflowRunEngineRepository,
};
use ora_domain::{WorkflowNodeStatus, WorkflowRunStatus};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// A Loop (at most three rounds, until `writer` answers `done`) whose body is
/// `entry → writer → checker`; `checker` never retries, so its failure fails the Loop.
fn loop_with_checker() -> String {
    let body_agent = |id: &str, extra: Value| {
        let mut node = agent(id, extra);
        node["parentId"] = json!("loop");
        node["data"]["containerId"] = json!("loop");
        node
    };
    let nodes = vec![
        json!({"id": "start", "data": {"kind": "start"}}),
        json!({"id": "loop", "data": {"kind": "loop", "loopConfig": {
            "maxIterations": 3,
            "variables": [{"name": "draft", "valueType": "string",
                "initial": {"kind": "constant", "value": "seed"}, "feedback": ["writer", "output"]}],
            "until": {"logic": "and", "conditions": [
                {"variableSelector": ["writer", "output"], "operator": "equals", "value": "done"}
            ]},
            "outputs": [{"name": "result", "variableSelector": ["writer", "output"]}]
        }}}),
        json!({"id": "entry", "parentId": "loop", "data": {"kind": "start", "containerId": "loop"}}),
        body_agent("writer", json!({})),
        body_agent("checker", retry(false, 0, 0)),
    ];
    let mut document: Value = serde_json::from_str(&graph(
        nodes,
        &[
            ("start", "loop"),
            ("entry", "writer"),
            ("writer", "checker"),
        ],
    ))
    .unwrap();
    document["schemaVersion"] = json!(2);
    document.to_string()
}

fn structured_output_failure(message: &str) -> NodeFailure {
    NodeFailure::new(NodeFailureKind::StructuredOutput, message)
        .with_output(Some("plain text".to_string()))
}

/// Replaces the frozen graph of the harness run, standing in for any error the engine hits
/// while it reads the run's execution context.
fn break_frozen_graph(h: &Harness) {
    rusqlite::Connection::open(h.temp.path().join("repository.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE workflow_snapshots SET graph = 'not json'
             WHERE id = (SELECT snapshot_id FROM workflow_runs WHERE id = ?1)",
            rusqlite::params![h.run_id.as_ref()],
        )
        .unwrap();
}

/// A failure an automatic retry already fixed stays fixed when a resume clears the successful
/// retry. The Loop reruns from round 1, and its fresh `writer` must not be told about round 1's
/// first attempt, while `checker`, whose failure was never fixed, is.
#[test]
fn a_failure_a_retry_fixed_is_not_injected_again_after_a_resume_clears_the_fix() {
    let h = Harness::start(&loop_with_checker());
    let first = h.running("writer");
    h.fail_with(
        &first.id,
        structured_output_failure("reply is not JSON"),
        1_000,
    );
    let fixed = h.running("writer");
    h.wake(&fixed.id, 11_000);
    assert!(
        h.prompt_for("writer")
            .contains("Previous attempt (1) failed")
    );
    h.complete(&fixed.id, "draft", 12_000);
    let checker = h.running("checker");
    h.fail_with(
        &checker.id,
        structured_output_failure("checker reply is not JSON"),
        13_000,
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);

    h.set_now(14_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let rerun = h.running("writer");
    assert_ne!(rerun.id, fixed.id);
    assert_eq!(
        h.repository()
            .find_last_failed_attempt(&h.run_id, "writer", rerun.iteration)
            .unwrap()
            .map(|row| row.id),
        None
    );
    let prompt = h.prompt_for("writer");
    assert!(!prompt.contains("Previous attempt"), "{prompt}");

    h.complete(&rerun.id, "draft", 15_000);
    let prompt = h.prompt_for("checker");
    assert!(prompt.contains("Previous attempt (1) failed"), "{prompt}");
    assert!(prompt.contains("checker reply is not JSON"), "{prompt}");
}

/// A restart during the wait fails the waiting row, which never ran. After a resume the new
/// attempt is still told about the attempt that really failed.
#[test]
fn a_restart_during_the_wait_keeps_the_real_failure_for_the_resumed_attempt() {
    let h = Harness::start(&linear(json!({})));
    h.fail_with(
        &h.running("a").id,
        structured_output_failure("reply is not JSON"),
        1_000,
    );
    let waiting = h.running("a");
    sweep_one_run(&h.repository(), &h.run_id, 5_000).unwrap();
    assert_eq!(
        (h.rows_of("a")[0].id.clone(), h.run().status),
        (waiting.id.clone(), WorkflowRunStatus::Failed)
    );

    h.set_now(20_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let prompt = h.prompt_for("a");
    assert_eq!(prompt.matches("Previous attempt").count(), 1, "{prompt}");
    assert!(prompt.contains("Previous attempt (1) failed"), "{prompt}");
    assert!(prompt.contains("reply is not JSON"), "{prompt}");
}

/// Once the wake has started the attempt, an error while resolving where to dispatch it fails
/// the attempt, so the run ends instead of keeping a `Running` row that nothing drives.
#[test]
fn a_wake_that_cannot_dispatch_the_started_attempt_fails_it_and_the_run() {
    let h = Harness::start(&linear(json!({})));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");
    break_frozen_graph(&h);

    h.wake(&waiting.id, 11_000);
    let row = h.rows_of("a").pop().unwrap();
    let kind = Harness::error_detail(&row)["kind"].clone();
    assert_eq!(
        (
            row.id,
            row.status,
            row.started_at,
            kind,
            h.run().status,
            h.dispatched("a").len(),
        ),
        (
            waiting.id,
            WorkflowNodeStatus::Failed,
            Some(11_000),
            json!("invalid_run_payload"),
            WorkflowRunStatus::Failed,
            1,
        )
    );
}

/// Inside a Loop round, a round state that no longer decodes is handled the same way: the writer
/// fails and the failure climbs to the Loop and the run.
#[test]
fn a_wake_that_cannot_read_its_loop_round_fails_the_attempt_and_the_run() {
    let h = Harness::start(&loop_with_checker());
    h.fail(&h.running("writer").id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("writer");
    rusqlite::Connection::open(h.temp.path().join("repository.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE workflow_execution_scopes SET state = '\"not a round state\"' WHERE id = ?1",
            rusqlite::params![waiting.scope_id.as_ref()],
        )
        .unwrap();

    h.wake(&waiting.id, 11_000);
    let row = h.rows_of("writer").pop().unwrap();
    let kind = Harness::error_detail(&row)["kind"].clone();
    assert_eq!(
        (row.id, row.status, kind, h.run().status,),
        (
            waiting.id,
            WorkflowNodeStatus::Failed,
            json!("invalid_run_payload"),
            WorkflowRunStatus::Failed,
        )
    );
}

/// An error while deciding whether to retry falls back to the normal failure path, so the node
/// fails instead of staying `Running` after its session ended.
#[test]
fn an_error_while_deciding_a_retry_falls_back_to_the_normal_failure() {
    let h = Harness::start(&linear(json!({})));
    let first = h.running("a");
    break_frozen_graph(&h);

    h.fail(&first.id, NodeFailureKind::Session, 1_000);
    let rows = h.rows_of("a");
    assert_eq!(
        rows.iter()
            .map(|row| (
                row.id.clone(),
                row.status,
                Harness::wait_of(row),
                Harness::error_detail(row)["kind"].clone()
            ))
            .collect::<Vec<_>>(),
        vec![(first.id, WorkflowNodeStatus::Failed, None, json!("session"))]
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
}

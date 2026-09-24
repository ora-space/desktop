//! Automatic retry inside Iteration and Loop rounds, and across a process restart.
//!
//! A retry stays inside the round of the attempt it replaces; the composite's own strategy only
//! ever sees the exhausted failure. The boot sweep treats a waiting attempt exactly like a
//! running one, so its timer never resumes it.

use super::recovery::sweep_one_run;
use super::retry_tests::{Dispatch, Harness, agent, graph, retry};
use ora_application::{
    NodeAutoRetry, NodeFailure, NodeFailureKind, ResumeWorkflowRunResult,
    WorkflowRunEngineRepository, WorkflowRunPayload,
};
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunStatus, WorkflowScopeStatus,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// `start → iter → out` over the Start array `prs`, whose region is the single agent `fix`,
/// optionally next to an outer agent `a`.
fn iteration_graph(error_strategy: &str, fix: Value, sibling: Option<Value>) -> String {
    let mut fix = agent("fix", fix);
    fix["parentId"] = json!("iter");
    let mut nodes = vec![
        json!({"id": "start", "data": {"kind": "start", "inputVariables": [
            {"name": "prs", "valueType": "array[object]"}
        ]}}),
        json!({"id": "iter", "data": {"kind": "iteration", "iterationConfig": {
            "iteratorSelector": ["start", "prs"],
            "collectSelector": ["fix", "output"],
            "errorStrategy": error_strategy,
            "maxIterations": 10
        }}}),
        fix,
        json!({"id": "out", "data": {"kind": "output", "outputs": [
            {"name": "collected", "variableSelector": ["iter", "output"]},
            {"name": "failedCount", "variableSelector": ["iter", "failed_count"]}
        ]}}),
    ];
    let mut edges = vec![("start", "iter"), ("iter", "fix"), ("iter", "out")];
    if let Some(sibling) = sibling {
        nodes.push(agent("a", sibling));
        edges.push(("start", "a"));
    }
    graph(nodes, &edges)
}

/// Starts a run of an iteration graph over three items.
fn iteration_run(graph: &str) -> Harness {
    let h = Harness::pending(graph);
    h.repository()
        .update_run_input(
            &h.run_id,
            Some("kickoff".to_string()),
            BTreeMap::from([("prs".to_string(), json!([{"id": 1}, {"id": 2}, {"id": 3}]))]),
            35,
        )
        .unwrap();
    h.engine.start(&h.run_id).unwrap();
    h
}

/// A Loop (at most three rounds, until `writer` answers `done`) whose body is `entry → writer`,
/// optionally next to an outer agent `a`.
pub(super) fn loop_graph(writer: Value, sibling: Option<Value>) -> String {
    let mut writer = agent("writer", writer);
    writer["parentId"] = json!("loop");
    writer["data"]["containerId"] = json!("loop");
    let mut nodes = vec![
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
        writer,
    ];
    let mut edges = vec![("start", "loop"), ("entry", "writer")];
    if let Some(sibling) = sibling {
        nodes.push(agent("a", sibling));
        edges.push(("start", "a"));
    }
    let mut document: Value = serde_json::from_str(&graph(nodes, &edges)).unwrap();
    document["schemaVersion"] = json!(2);
    document.to_string()
}

/// `(iteration, status)` of every live row of one node.
fn rounds(h: &Harness, node_id: &str) -> Vec<(Option<u32>, WorkflowNodeStatus)> {
    h.rows_of(node_id)
        .into_iter()
        .map(|row| (row.iteration, row.status))
        .collect()
}

/// The `iter.index` each dispatch of `fix` ran with, in dispatch order.
fn dispatched_rounds(h: &Harness) -> Vec<Value> {
    h.dispatches()
        .into_iter()
        .filter(|dispatch| dispatch.node_id == "fix")
        .map(|dispatch| dispatch.pool_values["iter.index"].clone())
        .collect()
}

fn run_payload(h: &Harness) -> WorkflowRunPayload {
    serde_json::from_str(h.run().payload.as_deref().unwrap()).unwrap()
}

fn connection(h: &Harness) -> rusqlite::Connection {
    rusqlite::Connection::open(h.temp.path().join("repository.sqlite3")).unwrap()
}

/// `(round_index, status)` of every round scope owned by one Loop row, oldest first.
fn loop_rounds(h: &Harness, loop_run_id: &WorkflowNodeRunId) -> Vec<(u32, WorkflowScopeStatus)> {
    let connection = connection(h);
    let mut statement = connection
        .prepare(
            "SELECT round_index, status FROM workflow_execution_scopes
             WHERE parent_loop_node_run_id = ?1 ORDER BY round_index",
        )
        .unwrap();
    statement
        .query_map(rusqlite::params![loop_run_id.as_ref()], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                WorkflowScopeStatus::from_database_value(row.get::<_, i64>(1)?).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Soft-deleted rows of one node in the harness run.
fn deleted_rows(h: &Harness, node_id: &str) -> i64 {
    connection(h)
        .query_row(
            "SELECT COUNT(*) FROM workflow_node_runs
             WHERE run_id = ?1 AND node_id = ?2 AND is_deleted = 1",
            rusqlite::params![h.run_id.as_ref(), node_id],
            |row| row.get(0),
        )
        .unwrap()
}

/// Test 11: a failed round is retried inside that round with the round's own item; rounds 1
/// and 3 never run again, under either error strategy.
#[test]
fn a_retry_inside_an_iteration_round_stays_in_that_round() {
    for strategy in ["fail", "continue"] {
        let h = iteration_run(&iteration_graph(strategy, json!({}), /*sibling*/ None));
        h.complete(&h.running("fix").id, "r0", 1_000);
        let failed = h.running("fix");
        assert_eq!(failed.iteration, Some(1));
        h.fail(&failed.id, NodeFailureKind::StructuredOutput, 2_000);

        let waiting = h.running("fix");
        let wait = Harness::wait_of(&waiting).unwrap();
        assert_eq!(
            (waiting.iteration, wait.attempt, wait.due_at),
            (Some(1), 2, 12_000)
        );
        assert_eq!(h.rows_of("iter")[0].status, WorkflowNodeStatus::Running);
        assert_eq!(dispatched_rounds(&h), vec![json!(0), json!(1)]);

        h.wake(&waiting.id, 12_000);
        let retried = h.dispatches().pop().unwrap();
        assert_eq!(
            (
                retried.node_run_id.as_str(),
                &retried.pool_values["iter.item"]
            ),
            (waiting.id.as_ref(), &json!({"id": 2}))
        );
        assert!(h.prompt_for("fix").contains("Previous attempt (1) failed"));
        h.complete(&waiting.id, "r1", 13_000);
        h.complete(&h.running("fix").id, "r2", 14_000);

        assert_eq!(
            (strategy, dispatched_rounds(&h)),
            (strategy, vec![json!(0), json!(1), json!(1), json!(2)])
        );
        assert_eq!(
            rounds(&h, "fix"),
            vec![
                (Some(0), WorkflowNodeStatus::Succeeded),
                (Some(1), WorkflowNodeStatus::Succeeded),
                (Some(2), WorkflowNodeStatus::Succeeded),
            ]
        );
        assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
        let pool = run_payload(&h).variable_pool.values;
        assert_eq!(
            (&pool["iter.output"], &pool["iter.failed_count"]),
            (&json!(["r0", "r1", "r2"]), &json!(0))
        );
    }
}

/// Test 11: only the exhausted failure reaches the iteration strategy — `fail` stops the node
/// and the run, `continue` absorbs the round and runs the next one.
#[test]
fn an_exhausted_retry_inside_an_iteration_round_follows_the_error_strategy() {
    let h = iteration_run(&iteration_graph(
        "fail",
        retry(true, 1, 0),
        /*sibling*/ None,
    ));
    h.complete(&h.running("fix").id, "r0", 1_000);
    h.fail(&h.running("fix").id, NodeFailureKind::Session, 2_000);
    h.wake(&h.running("fix").id, 2_000);
    h.fail(&h.running("fix").id, NodeFailureKind::Session, 3_000);
    assert_eq!(
        rounds(&h, "fix"),
        vec![
            (Some(0), WorkflowNodeStatus::Succeeded),
            (Some(1), WorkflowNodeStatus::Failed),
        ]
    );
    assert_eq!(
        Harness::error_detail(&h.rows_of("fix")[1])["attempt"],
        json!(2)
    );
    assert_eq!(dispatched_rounds(&h), vec![json!(0), json!(1), json!(1)]);
    assert_eq!(
        (h.rows_of("iter")[0].status, h.run().status),
        (WorkflowNodeStatus::Failed, WorkflowRunStatus::Failed)
    );

    let h = iteration_run(&iteration_graph(
        "continue",
        retry(true, 1, 0),
        /*sibling*/ None,
    ));
    h.complete(&h.running("fix").id, "r0", 1_000);
    h.fail(&h.running("fix").id, NodeFailureKind::Session, 2_000);
    h.wake(&h.running("fix").id, 2_000);
    h.fail(&h.running("fix").id, NodeFailureKind::Session, 3_000);
    assert_eq!(
        dispatched_rounds(&h),
        vec![json!(0), json!(1), json!(1), json!(2)]
    );
    h.complete(&h.running("fix").id, "r2", 4_000);
    assert_eq!(
        rounds(&h, "fix"),
        vec![
            (Some(0), WorkflowNodeStatus::Succeeded),
            (Some(1), WorkflowNodeStatus::Failed),
            (Some(2), WorkflowNodeStatus::Succeeded),
        ]
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
    let pool = run_payload(&h).variable_pool.values;
    assert_eq!(
        (&pool["iter.output"], &pool["iter.failed_count"]),
        (&json!(["r0", "r2"]), &json!(1))
    );
}

/// Test 9 (outer node): a restart during the wait interrupts the waiting attempt and fails the
/// run like a running node; its timer then starts nothing, and a resume starts a fresh budget
/// while attempt numbers keep counting.
#[test]
fn a_restart_during_the_wait_interrupts_an_outer_node_and_fails_the_run() {
    let h = Harness::start(&graph(
        vec![
            json!({"id": "start", "data": {"kind": "start"}}),
            agent("a", json!({})),
        ],
        &[("start", "a")],
    ));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");

    sweep_one_run(&h.repository(), &h.run_id, 5_000).unwrap();
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    let interrupted = h.rows_of("a").pop().unwrap();
    let detail = Harness::error_detail(&interrupted);
    assert_eq!(
        (
            &interrupted.id,
            interrupted.status,
            Harness::wait_of(&interrupted),
            detail["kind"].clone(),
            detail["attempt"].clone(),
            detail["resumable"].clone(),
        ),
        (
            &waiting.id,
            WorkflowNodeStatus::Failed,
            None,
            json!("interrupted_by_restart"),
            json!(2),
            json!(true),
        )
    );

    h.wake(&waiting.id, 11_000);
    assert_eq!(h.dispatched("a").len(), 1);

    h.set_now(20_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let resumed = h.running("a");
    assert_eq!(
        NodeAutoRetry::from_payload(resumed.payload.as_deref()),
        None
    );
    h.fail(&resumed.id, NodeFailureKind::Session, 21_000);
    let wait = Harness::wait_of(&h.running("a")).unwrap();
    assert_eq!(
        (wait.attempt, wait.max_attempt, wait.retry, wait.delay_ms),
        (4, 5, 1, 10_000)
    );
}

/// Test 9 (inside an iteration): a restart during the wait is absorbed by the running
/// iteration — the round settles as failed, the next round runs — and the timer starts nothing.
#[test]
fn a_restart_during_the_wait_inside_an_iteration_is_absorbed_by_the_round() {
    let h = iteration_run(&iteration_graph(
        "continue",
        json!({}),
        /*sibling*/ None,
    ));
    h.complete(&h.running("fix").id, "r0", 1_000);
    h.fail(&h.running("fix").id, NodeFailureKind::Session, 2_000);
    let waiting = h.running("fix");

    sweep_one_run(&h.repository(), &h.run_id, 5_000).unwrap();
    assert_eq!(h.run().status, WorkflowRunStatus::Running);
    let interrupted = h
        .rows_of("fix")
        .into_iter()
        .find(|row| row.id == waiting.id)
        .unwrap();
    assert_eq!(
        (
            interrupted.status,
            Harness::wait_of(&interrupted),
            interrupted.error.as_deref()
        ),
        (
            WorkflowNodeStatus::Failed,
            None,
            Some(r#"{"reason":"interrupted_by_restart"}"#)
        )
    );
    // Boot reconcile resumes scheduling of a surviving run with no generating row.
    h.engine.resume(&h.run_id).unwrap();
    h.wake(&waiting.id, 12_000);
    assert_eq!(dispatched_rounds(&h), vec![json!(0), json!(1), json!(2)]);

    h.complete(&h.running("fix").id, "r2", 13_000);
    assert_eq!(
        rounds(&h, "fix"),
        vec![
            (Some(0), WorkflowNodeStatus::Succeeded),
            (Some(1), WorkflowNodeStatus::Failed),
            (Some(2), WorkflowNodeStatus::Succeeded),
        ]
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        run_payload(&h).variable_pool.values["iter.failed_count"],
        json!(1)
    );
}

/// Test 12: a Loop body agent retries inside its round, against the Loop body and that round's
/// isolated pool; the next round neither reruns the retry nor inherits its failure block.
#[test]
fn a_retry_inside_a_loop_round_stays_in_that_round_and_later_rounds_inherit_nothing() {
    let h = Harness::start(&loop_graph(json!({}), /*sibling*/ None));
    let first = h.running("writer");
    h.fail(&first.id, NodeFailureKind::StructuredOutput, 1_000);
    let waiting = h.running("writer");
    assert_eq!(waiting.scope_id, first.scope_id);
    assert_eq!(h.rows_of("loop")[0].status, WorkflowNodeStatus::Running);

    h.wake(&waiting.id, 11_000);
    let dispatches: Vec<Dispatch> = h
        .dispatches()
        .into_iter()
        .filter(|dispatch| dispatch.node_id == "writer")
        .collect();
    assert_eq!(dispatches.len(), 2);
    assert_eq!(
        (
            &dispatches[1].scope_id,
            &dispatches[1].graph_nodes,
            &dispatches[1].pool_values
        ),
        (
            &first.scope_id,
            &vec!["entry".to_string(), "writer".to_string()],
            &dispatches[0].pool_values
        )
    );
    assert!(
        h.prompt_for("writer")
            .contains("Previous attempt (1) failed")
    );

    h.complete(&waiting.id, "draft one", 12_000);
    let round_two = h.running("writer");
    assert_ne!(round_two.scope_id, first.scope_id);
    assert_eq!(h.dispatched("writer").len(), 3);
    assert!(!h.prompt_for("writer").contains("Previous attempt"));

    h.complete(&round_two.id, "done", 13_000);
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
    assert_eq!(h.rows_of("loop")[0].status, WorkflowNodeStatus::Succeeded);
}

/// Both the retry budget and the attempt numbers belong to the Loop round: a writer that used one
/// of its two retries in round 1 starts round 2 at attempt 1 with both retries, and only a third
/// failure in one round would exhaust it. The failure history and the block injected into a
/// retry use the same round-local numbers.
#[test]
fn a_loop_member_gets_its_full_retry_budget_and_fresh_attempt_numbers_in_every_round() {
    let h = Harness::start(&loop_graph(retry(true, 2, 1), /*sibling*/ None));
    let wait_tuple = |row: &WorkflowNodeRun| {
        let wait = Harness::wait_of(row).unwrap();
        (
            wait.attempt,
            wait.max_attempt,
            wait.retry,
            wait.max_retries,
            wait.delay_ms,
        )
    };

    let round_one = h.running("writer");
    h.fail(&round_one.id, NodeFailureKind::Session, 1_000);
    let round_one_retry = h.running("writer");
    assert_eq!(wait_tuple(&round_one_retry), (2, 3, 1, 2, 1_000));
    h.wake(&round_one_retry.id, 2_000);
    h.complete(&round_one_retry.id, "again", 3_000);

    let round_two = h.running("writer");
    assert_ne!(round_two.scope_id, round_one.scope_id);
    assert_eq!(
        NodeAutoRetry::from_payload(round_two.payload.as_deref()),
        None
    );
    h.fail(&round_two.id, NodeFailureKind::Session, 4_000);
    let first_retry = h.running("writer");
    // Attempt 2 of round 2: retry 1 of a fresh budget of 2 after the shorter first wait.
    assert_eq!(wait_tuple(&first_retry), (2, 3, 1, 2, 1_000));
    h.wake(&first_retry.id, 5_000);
    h.fail_with(
        &first_retry.id,
        NodeFailure::new(
            NodeFailureKind::StructuredOutput,
            "round two reply is not JSON",
        ),
        6_000,
    );
    let second_retry = h.running("writer");
    assert_eq!(wait_tuple(&second_retry), (3, 3, 2, 2, 2_000));
    h.wake(&second_retry.id, 8_000);
    let prompt = h.prompt_for("writer");
    assert!(prompt.contains("Previous attempt (2) failed"), "{prompt}");
    assert!(prompt.contains("round two reply is not JSON"), "{prompt}");
    assert_eq!(h.run().status, WorkflowRunStatus::Running);
    h.complete(&second_retry.id, "done", 9_000);

    assert_eq!(h.rows_of("loop")[0].status, WorkflowNodeStatus::Succeeded);
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        h.detail()
            .failed_attempts
            .iter()
            .map(|row| (
                row.scope_id.clone(),
                Harness::error_detail(row)["attempt"].clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (round_one.scope_id.clone(), json!(1)),
            (round_two.scope_id.clone(), json!(1)),
            (round_two.scope_id.clone(), json!(2)),
        ]
    );
}

/// A Loop resume reruns the Loop from round 1 in new round scopes, so its body attempts number
/// from 1 again instead of continuing from the rounds the resume cleared.
#[test]
fn a_loop_resume_numbers_body_attempts_from_one_again() {
    let h = Harness::start(&loop_graph(retry(true, 1, 0), /*sibling*/ None));
    h.fail(&h.running("writer").id, NodeFailureKind::Session, 1_000);
    let retry_row = h.running("writer");
    assert_eq!(Harness::wait_of(&retry_row).unwrap().attempt, 2);
    h.wake(&retry_row.id, 2_000);
    h.fail(&retry_row.id, NodeFailureKind::Session, 3_000);
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);

    h.set_now(4_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let rerun = h.running("writer");
    assert_ne!(rerun.scope_id, retry_row.scope_id);
    h.fail(&rerun.id, NodeFailureKind::Session, 5_000);
    let rerun_retry = h.running("writer");
    let wait = Harness::wait_of(&rerun_retry).unwrap();
    assert_eq!((wait.attempt, wait.max_attempt, wait.retry), (2, 2, 1));
    h.wake(&rerun_retry.id, 6_000);
    h.fail(&rerun_retry.id, NodeFailureKind::Session, 7_000);
    assert_eq!(
        (
            Harness::error_detail(&h.rows_of("writer").pop().unwrap())["attempt"].clone(),
            h.run().status
        ),
        (json!(2), WorkflowRunStatus::Failed)
    );
    assert_eq!(
        h.detail()
            .failed_attempts
            .iter()
            .filter(|row| row.node_id == "writer")
            .map(|row| (
                row.scope_id.clone(),
                Harness::error_detail(row)["attempt"].clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (retry_row.scope_id.clone(), json!(1)),
            (retry_row.scope_id.clone(), json!(2)),
            (rerun.scope_id.clone(), json!(1)),
        ]
    );
}

/// Test 12: an exhausted Loop body agent keeps Loop semantics — the Loop and the run fail — and
/// the run failure abandons a retry waiting elsewhere, which resume reruns.
#[test]
fn an_exhausted_retry_inside_a_loop_fails_the_loop_and_abandons_other_waits() {
    let h = Harness::start(&loop_graph(retry(true, 1, 0), Some(retry(true, 1, 30))));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let waiting_a = h.running("a");
    h.fail(&h.running("writer").id, NodeFailureKind::Session, 2_000);
    h.wake(&h.running("writer").id, 2_000);
    h.fail(&h.running("writer").id, NodeFailureKind::Session, 3_000);

    assert_eq!(
        (
            h.rows_of("writer")[0].status,
            h.rows_of("loop")[0].status,
            h.run().status
        ),
        (
            WorkflowNodeStatus::Failed,
            WorkflowNodeStatus::Failed,
            WorkflowRunStatus::Failed
        )
    );
    let abandoned = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (&abandoned.id, abandoned.status, abandoned.error.as_deref()),
        (
            &waiting_a.id,
            WorkflowNodeStatus::Cancelled,
            Some(r#"{"reason":"retry_abandoned"}"#)
        )
    );
    h.wake(&waiting_a.id, 31_000);
    assert_eq!(h.dispatched("a").len(), 1);

    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    assert_eq!(
        (h.dispatched("a").len(), h.dispatched("writer").len()),
        (2, 3)
    );
}

/// Test 7 across scopes: an outer node's final failure fails the run while a Loop body retry
/// waits in round 2. The Loop row keeps running as every in-flight outer sibling does, but the
/// waiting attempt is abandoned so its timer starts nothing. The abandoned body row makes the
/// Loop the resume unit, exactly as for an Iteration: the preview offers the resume with the
/// Loop as unit, and resume closes both rounds, soft-deletes their rows, and restarts the Loop
/// from round 1 with a fresh budget instead of resuming inside the open round.
#[test]
fn an_outer_final_failure_abandons_a_loop_wait_and_resume_restarts_the_loop_from_round_one() {
    let h = Harness::start(&loop_graph(json!({}), Some(retry(false, 0, 0))));
    let loop_old = h.rows_of("loop")[0].id.clone();
    let round_one = h.running("writer");
    h.complete(&round_one.id, "again", 500);
    let round_two = h.running("writer");
    assert_ne!(round_two.scope_id, round_one.scope_id);
    h.fail(&round_two.id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("writer");
    h.fail(&h.running("a").id, NodeFailureKind::Session, 2_000);

    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    let abandoned = h.rows_of("writer").pop().unwrap();
    assert_eq!(
        (
            &abandoned.id,
            abandoned.status,
            Harness::wait_of(&abandoned),
            abandoned.error.as_deref()
        ),
        (
            &waiting.id,
            WorkflowNodeStatus::Cancelled,
            None,
            Some(r#"{"reason":"retry_abandoned"}"#)
        )
    );
    assert_eq!(h.rows_of("loop")[0].status, WorkflowNodeStatus::Running);
    assert_eq!(
        loop_rounds(&h, &loop_old),
        vec![
            (1, WorkflowScopeStatus::Succeeded),
            (2, WorkflowScopeStatus::Running)
        ]
    );
    h.wake(&waiting.id, 11_000);
    assert_eq!(h.dispatched("writer").len(), 2);

    // The parked Loop row does not block a resume, and the preview agrees with the engine.
    let preview = super::rollback::preview(&h.pool, h.temp.path(), &h.run_id).unwrap();
    assert!(preview.resumable, "{preview:?}");
    assert_eq!(
        preview
            .failed_nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node.resume_unit_node_id.as_deref()))
            .collect::<Vec<_>>(),
        vec![("a", None), ("writer", Some("loop"))]
    );
    assert_eq!(
        preview.node_files_unavailable_reason.as_deref(),
        Some("composite_region")
    );

    h.set_now(20_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let loop_new = h.rows_of("loop").pop().unwrap();
    assert_ne!(loop_new.id, loop_old);
    assert_eq!(loop_new.status, WorkflowNodeStatus::Running);
    assert_eq!(
        loop_rounds(&h, &loop_old),
        vec![
            (1, WorkflowScopeStatus::Succeeded),
            (2, WorkflowScopeStatus::Cancelled)
        ]
    );
    assert_eq!(
        loop_rounds(&h, &loop_new.id),
        vec![(1, WorkflowScopeStatus::Running)]
    );
    // Round 1's success, round 2's replaced attempt and its abandoned wait are all history.
    assert_eq!(
        (deleted_rows(&h, "writer"), deleted_rows(&h, "entry")),
        (3, 2)
    );
    let writer = h.running("writer");
    assert!(writer.scope_id != round_one.scope_id && writer.scope_id != round_two.scope_id);
    assert_eq!(NodeAutoRetry::from_payload(writer.payload.as_deref()), None);
    assert_eq!(
        (h.dispatched("a").len(), h.dispatched("writer").len()),
        (2, 3)
    );

    h.complete(&writer.id, "done", 21_000);
    h.complete(&h.running("a").id, "a done", 22_000);
    assert_eq!(h.rows_of("loop")[0].status, WorkflowNodeStatus::Succeeded);
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
}

/// Test 7 inside an iteration: an outer node's final failure fails the run while a region
/// retry waits. The iteration row keeps running as every in-flight outer sibling does, the
/// waiting attempt is abandoned, and resume restarts the iteration as a whole (a cancelled
/// region row makes the owning composite the resume unit).
#[test]
fn an_outer_final_failure_abandons_a_retry_waiting_inside_an_iteration() {
    let h = iteration_run(&iteration_graph(
        "fail",
        json!({}),
        Some(retry(false, 0, 0)),
    ));
    h.complete(&h.running("fix").id, "r0", 1_000);
    h.fail(&h.running("fix").id, NodeFailureKind::Session, 2_000);
    let waiting = h.running("fix");
    h.fail(&h.running("a").id, NodeFailureKind::Session, 3_000);

    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    let abandoned = h
        .rows_of("fix")
        .into_iter()
        .find(|row| row.id == waiting.id)
        .unwrap();
    assert_eq!(
        (abandoned.status, abandoned.error.as_deref()),
        (
            WorkflowNodeStatus::Cancelled,
            Some(r#"{"reason":"retry_abandoned"}"#)
        )
    );
    h.wake(&waiting.id, 12_000);
    assert_eq!(dispatched_rounds(&h), vec![json!(0), json!(1)]);

    h.set_now(20_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    assert_eq!(dispatched_rounds(&h), vec![json!(0), json!(1), json!(0)]);
    h.complete(&h.running("a").id, "a done", 21_000);
    for (round, at) in [(0, 22_000), (1, 23_000), (2, 24_000)] {
        h.complete(&h.running("fix").id, &format!("r{round}"), at);
    }
    assert_eq!(h.run().status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        run_payload(&h).variable_pool.values["iter.output"],
        json!(["r0", "r1", "r2"])
    );
}

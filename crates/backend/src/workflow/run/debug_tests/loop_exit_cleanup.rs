//! Preview a rewind before and after an explicit exit's asynchronous cleanup.
use super::super::{
    debug::strategy,
    loop_exit_tests,
    test_fixture::{AGENT_GRAPH, started_run_with},
};
use super::*;
use ora_contracts::{PublicError, WorkflowDebugTargetRejectedParams};
use ora_db::workflow_debug::DebugRejection;
use pretty_assertions::assert_eq;
use serde_json::Value;

/// Put a selectable root agent upstream of the Loop so that both planner paths see the same exit.
fn graph() -> String {
    let mut graph: Value = serde_json::from_str(&loop_exit_tests::graph(true)).unwrap();
    let agent: Value = serde_json::from_str(AGENT_GRAPH).unwrap();
    graph["nodes"]
        .as_array_mut()
        .unwrap()
        .push(agent["nodes"][1].clone());
    graph["edges"][0]["target"] = json!("agent");
    graph["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":"agent","target":"loop"}));
    graph.to_string()
}

/// Drive the real exit and check both the planner refusal and the reason shown by Desktop.
fn check_exit_rewind(outcome: WorkflowRunStatus) {
    let (temp, pool) = bootstrap();
    let (run, initial, engine) =
        started_run_with(&temp, &pool, &graph(), loop_exit_tests::PendingCleanup);
    let repo = SqliteWorkflowRunRepository::new(pool.clone());
    let store = WorkflowDebugStore::new(pool.clone());
    let upstream = initial.iter().find(|r| r.node_id == "agent").unwrap();
    let initial_run = repo.find_run(&run).unwrap().unwrap();
    let payload: ora_application::WorkflowRunPayload =
        serde_json::from_str(initial_run.payload.as_deref().unwrap()).unwrap();
    assert!(
        store
            .record_agent_dispatch(
                run.as_ref(),
                upstream.id.as_ref(),
                &json!({"variablePool":payload.variable_pool})
            )
            .unwrap()
    );
    engine
        .complete_node(&run, &upstream.id, Some("ready".into()), None, None, vec![])
        .unwrap();
    let members = node_runs(&pool, &run);
    let writer = members.iter().find(|r| r.node_id == "writer").unwrap();
    let owner = members.iter().find(|r| r.node_id == "loop").unwrap();
    engine
        .complete_node(&run, &writer.id, Some("done".into()), None, None, vec![])
        .unwrap();
    let rows = node_runs(&pool, &run);
    let check = rows.iter().find(|r| r.node_id == "check").unwrap();
    assert!(
        rows.iter()
            .any(|r| r.node_id == "slow" && r.status == WorkflowNodeStatus::Cancelled)
    );
    let requests = [
        (&check.id, "/cases/0/conditions/0/value", json!("continue")),
        (&owner.id, "/loopConfig/maxIterations", json!(3)),
        (&upstream.id, "/agentConfig/prompt", json!("revised")),
    ]
    .map(|(id, path, value)| {
        serde_json::from_value(json!({
            "runId":run, "nodeRunId":id, "baseRevision":0,
            "patch":[{"op":"replace","path":path,"value":value}]
        }))
        .unwrap()
    });
    let assert_rejected = |expected: DebugRejection, guidance: &str| {
        let before = repo.get_run_detail(&run).unwrap().unwrap();
        for request in &requests {
            let error = store.preview(request, strategy::plan).unwrap_err();
            assert!(
                matches!(&error, ora_db::DatabaseError::WorkflowDebug(reason) if reason == &expected),
                "{error}"
            );
            let error = BackendError::from_debug_store(error);
            assert!(
                matches!(error.public_error(), PublicError::WorkflowDebugTargetRejected(
                WorkflowDebugTargetRejectedParams { reason, .. }) if reason.contains(guidance)),
                "{error:?}"
            );
        }
        assert!(store.history(run.as_ref()).unwrap().is_empty());
        assert_eq!(repo.get_run_detail(&run).unwrap().unwrap(), before);
    };
    assert_rejected(
        DebugRejection::LoopExitCleanupPending,
        "Wait for it to finish",
    );
    match outcome {
        WorkflowRunStatus::Cancelled => {
            engine.cancel(&run).unwrap();
            // A late successful receipt cannot establish isolation for a cancelled round.
            engine
                .finish_loop_exit(&run, &owner.id, &writer.scope_id, Ok(()))
                .unwrap();
        }
        WorkflowRunStatus::Failed => engine
            .finish_loop_exit(
                &run,
                &owner.id,
                &writer.scope_id,
                Err("session stop failed".into()),
            )
            .unwrap(),
        WorkflowRunStatus::Succeeded => engine
            .finish_loop_exit(&run, &owner.id, &writer.scope_id, Ok(()))
            .unwrap(),
        _ => unreachable!("expected a terminal run"),
    }
    assert_eq!(repo.find_run(&run).unwrap().unwrap().status, outcome);
    if outcome == WorkflowRunStatus::Succeeded {
        for request in &requests {
            store.preview(request, strategy::plan).unwrap();
        }
    } else {
        assert_rejected(
            DebugRejection::LoopExitRecoveryRequired,
            "cancelled siblings stopped. Use ordinary failure recovery",
        );
    }
}

/// A successful receipt releases the pending exit.
#[test]
fn exit_rewind_waits_for_the_cleanup_acknowledgement() {
    run_test(async { check_exit_rewind(WorkflowRunStatus::Succeeded) });
}

/// Cancelling before the receipt leaves ordinary recovery as the way to resume.
#[test]
fn exit_rewind_requires_recovery_after_run_cancellation() {
    run_test(async { check_exit_rewind(WorkflowRunStatus::Cancelled) });
}

/// A failed stop cannot be retried by previewing the edit.
#[test]
fn exit_rewind_requires_recovery_after_cleanup_failure() {
    run_test(async { check_exit_rewind(WorkflowRunStatus::Failed) });
}

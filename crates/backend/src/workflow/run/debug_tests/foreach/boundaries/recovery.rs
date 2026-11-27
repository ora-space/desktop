use super::*;
use pretty_assertions::assert_eq;

/// A completed owner must reopen on an explicit recovery after the run failed during quiescence.
#[test]
fn ordinary_resume_replays_a_parked_member_before_dispatching_its_consumer() {
    run_test(park_and_recover(false, false));
}

#[test]
fn ordinary_resume_preserves_a_settled_failure_in_the_replay_prefix() {
    run_test(park_and_recover(true, false));
}

#[test]
fn editing_the_failed_sibling_keeps_the_reactivated_foreach_in_scope() {
    run_test(park_and_recover(false, true));
}

async fn park_and_recover(failed_prefix: bool, debug_sibling: bool) {
    let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
    let f = batch_graph(barrier.clone(), true);
    let owner = active(&f, "iter").id;
    let first_row = active(&f, "member").id;
    if failed_prefix {
        f.engine
            .fail_node(
                &f.run_id,
                &first_row,
                NodeFailure::new(NodeFailureKind::Session, "absorbed"),
            )
            .unwrap();
    } else {
        complete(&f, "member", "one");
    }
    let sibling = active(&f, "sibling");
    evidence(&f, &sibling, &payload(&f).variable_pool);
    complete(&f, "member", "two");
    let third = active(&f, "member");
    evidence(&f, &third, &payload(&f).variable_pool);
    for value in ["old3", "old4", "old5"] {
        complete(&f, "member", value);
    }
    let consumer = active(&f, "consumer");
    bind(&f, &consumer);
    let preview = f.preview(third.id.as_ref(), 0);
    let (op, ()) = tokio::join!(
        f.coordinator.commit(f.commit_request(&preview, "park")),
        async {
            barrier.entered.notified().await;
            f.engine
                .fail_node(
                    &f.run_id,
                    &active(&f, "sibling").id,
                    NodeFailure::new(NodeFailureKind::Session, "independent"),
                )
                .unwrap();
            barrier.release.notify_one();
        }
    );
    assert_eq!(
        op.unwrap().phase,
        WorkflowDebugPhase::Applied { revision: 1 }
    );
    assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
    assert_eq!(f.row("iter").status, WorkflowNodeStatus::Succeeded);
    assert_eq!(
        payload(&f).variable_pool.values["iter.output"],
        if failed_prefix {
            json!(["two", "old3", "old4", "old5"])
        } else {
            json!(["one", "two", "old3", "old4", "old5"])
        }
    );
    assert_eq!(dispatched(&f.executor, "member"), 5);
    if debug_sibling {
        let edit = f.preview(sibling.id.as_ref(), 1);
        for node in ["sibling", "iter", "member", "next", "consumer"] {
            assert!(edit.impact.schedule.iter().any(|id| id == node));
        }
        f.coordinator
            .commit(f.commit_request(&edit, "fix-independent"))
            .await
            .unwrap();
        complete(&f, "sibling", "recovered");
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
    } else {
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
    }
    assert_eq!(active(&f, "iter").id, owner);
    for (index, item, value) in [(2, "c", "new3"), (3, "d", "new4"), (4, "e", "new5")] {
        let row = active(&f, "member");
        assert_eq!(row.iteration, Some(index));
        assert_eq!(payload(&f).variable_pool.values["iter.item"], json!(item));
        assert!(!payload(&f).variable_pool.values.contains_key("iter.output"));
        assert_eq!(dispatched(&f.executor, "consumer"), 1);
        let graph = f
            .store
            .revised_graph(f.run_id.as_ref(), row.id.as_ref())
            .unwrap()
            .unwrap();
        assert_eq!(
            graph
                .node("member")
                .unwrap()
                .agent_config
                .as_ref()
                .unwrap()
                .prompt,
            "new"
        );
        complete(&f, "member", value);
    }
    assert_eq!(dispatched(&f.executor, "member"), 8);
    let records = f.executor.records();
    let dispatch = records
        .iter()
        .rev()
        .find(|r| r.node_id == "consumer")
        .unwrap();
    let input: WorkflowRunPayload =
        serde_json::from_str(dispatch.payload.as_deref().unwrap()).unwrap();
    assert_eq!(
        input.variable_pool.values["iter.output"],
        if failed_prefix {
            json!(["two", "new3", "new4", "new5"])
        } else {
            json!(["one", "two", "new3", "new4", "new5"])
        }
    );
    if !debug_sibling {
        complete(&f, "sibling", "recovered");
    }
    assert_eq!(
        payload(&f).variable_pool.values["iter.failed_count"],
        json!(u32::from(failed_prefix))
    );
    assert!(
        node_runs(&f.pool, &f.run_id)
            .iter()
            .any(|row| row.id == first_row)
    );
    for _ in input.variable_pool.values["iter.output"]
        .as_array()
        .unwrap()
    {
        complete(&f, "consumer", "ok");
    }
    assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
}

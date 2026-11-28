use super::*;
use pretty_assertions::assert_eq;

/// Without a parked edit, downstream failure restarts the entire failed member's container.
#[test]
fn ordinary_resume_retries_an_absorbed_failure_without_a_debug_edit() {
    run_test(async {
        let f = batch_graph(Arc::new(NoSessions), true);
        let owner = active(&f, "iter").id;
        complete(&f, "sibling", "ok");
        let fail = |node: &str| {
            f.engine
                .fail_node(
                    &f.run_id,
                    &active(&f, node).id,
                    NodeFailure::new(NodeFailureKind::Session, "failed"),
                )
                .unwrap();
        };
        fail("member");
        for value in ["two", "three", "four", "five"] {
            complete(&f, "member", value);
        }
        assert_eq!(
            payload(&f).variable_pool.values["iter.failed_count"],
            json!(1)
        );
        fail("consumer");
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(dispatched(&f.executor, "member"), 6);
        assert_ne!(active(&f, "iter").id, owner);
        assert_eq!(active(&f, "member").iteration, Some(0));
        let expected = ["one", "two", "three", "four", "five"];
        for value in expected {
            complete(&f, "member", value);
        }
        let values = payload(&f).variable_pool.values;
        assert_eq!(values["iter.failed_count"], json!(0));
        assert_eq!(values["iter.output"], json!(expected));
        for value in expected {
            complete(&f, "consumer", value);
        }
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
    });
}

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

/// A projection failure after real quiescence cannot replace an independently committed outcome.
#[test]
fn projection_failure_preserves_the_independent_error_and_finish_time() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = batch_graph(barrier.clone(), true);
        let target = active(&f, "member");
        bind(&f, &target);
        evidence(&f, &target, &payload(&f).variable_pool);
        let preview = f.preview(target.id.as_ref(), 0);
        let load = || {
            SqliteWorkflowRunRepository::new(f.pool.clone())
                .find_run(&f.run_id)
                .unwrap()
                .unwrap()
        };
        let (receipt, prior) = tokio::join!(
            f.coordinator
                .commit(f.commit_request(&preview, "lost-evidence")),
            async {
                barrier.entered.notified().await;
                f.engine
                    .fail_node(
                        &f.run_id,
                        &active(&f, "sibling").id,
                        NodeFailure::new(NodeFailureKind::Session, "independent"),
                    )
                    .unwrap();
                rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap().execute(
                "DELETE FROM workflow_debug_documents WHERE node_run_id=?1 AND kind='dispatch'",[target.id.as_ref()]).unwrap();
                let prior = load();
                barrier.release.notify_one();
                prior
            }
        );
        assert!(
            matches!(receipt.unwrap().phase,WorkflowDebugPhase::StopFailed{ reason,.. } if reason.contains("missing_input_evidence"))
        );
        let after = load();
        assert_eq!(after.status, WorkflowRunStatus::Failed);
        assert_eq!(
            (after.error, after.finished_at),
            (prior.error, prior.finished_at)
        );
        assert_eq!(dispatched(&f.executor, "member"), 1);
    });
}

/// A parked Condition edit retires its own item's route but preserves the settled prefix.
#[test]
fn terminal_condition_replay_keeps_only_prior_item_routes() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_inputs(
            barrier.clone(),
            &route_graph(),
            [("items".into(), json!(["a", "a"]))].into(),
        );
        complete(&f, "fix", "first");
        let fix = active(&f, "fix");
        bind(&f, &fix);
        let gate = node_runs(&f.pool, &f.run_id)
            .into_iter()
            .find(|r| r.node_id == "gate" && r.iteration == Some(1))
            .unwrap();
        let preview = f.coordinator.preview(serde_json::from_value(json!({"runId":f.run_id.as_ref(),"nodeRunId":gate.id.as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/cases/0/conditions/0/value","value":"b"}]})).unwrap()).unwrap();
        let before = payload(&f).iteration_condition_decisions;
        assert_eq!(before["gate#1"], "match");
        let (op, ()) = tokio::join!(
            f.coordinator
                .commit(f.commit_request(&preview, "retire-route")),
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
        let routes = payload(&f).iteration_condition_decisions;
        assert_eq!(routes["gate#0"], before["gate#0"]);
        assert!(!routes.contains_key("gate#1"));
    });
}

//! Review findings on desktop#871: a parked edit made on top of an earlier edit's archived
//! slots (P1-1), and a foreach entrance edit applied to a terminal run (P2-1). In both, the
//! first dispatch after ordinary recovery must not read an output of a retired attempt.
use super::*;
use pretty_assertions::assert_eq;

/// P1-1: edit round 3 so round 4 is archived awaiting replay; while round 3 reruns, edit round
/// 2 and let `outside` fail during the stop. Ordinary recovery replays round 2 with the newer
/// revision and round 3 reads its new output, never the retired S2. Round 4 keeps the first
/// edit as the operation that archived it.
#[test]
fn a_parked_edit_over_an_earlier_archived_slot_still_replays_after_recovery() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        f.finish_writer("S1");
        let round_two = f.observe_live_writer();
        f.finish_writer("S2");
        let round_three = f.observe_live_writer();
        f.finish_writer("S3");
        let round_four = f.observe_live_writer();
        f.bind_agent(&WorkflowNodeRunId::new(&round_four.node_run_id));
        let first = f.prompt_preview(&round_three.node_run_id, 0, "new");
        assert_eq!(first.impact.stop, vec![round_four.node_run_id.clone()]);
        let (op1, ()) = tokio::join!(
            f.coordinator.commit(f.commit_request(&first, "first")),
            async {
                barrier.entered.notified().await;
                barrier.release.notify_one();
            }
        );
        let op1 = op1.unwrap();
        assert_eq!(op1.phase, WorkflowDebugPhase::Applied { revision: 1 });
        let rerun = f.observe_live_writer();
        assert_eq!(
            (
                rerun.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&rerun.node_run_id)
            ),
            (Some("S2"), f.scope_of(&round_three.node_run_id))
        );
        f.bind_agent(&WorkflowNodeRunId::new(&rerun.node_run_id));

        let second = f.prompt_preview(&round_two.node_run_id, 1, "new2");
        assert_eq!(
            (second.target.round, second.impact.stop.clone()),
            (Some(2), vec![rerun.node_run_id.clone()])
        );
        let (op2, ()) = tokio::join!(
            f.coordinator.commit(f.commit_request(&second, "second")),
            async {
                barrier.entered.notified().await;
                f.engine
                    .fail_node(
                        &f.run_id,
                        &f.row("outside").id,
                        ora_application::NodeFailure::new(
                            ora_application::NodeFailureKind::Session,
                            "outside failed",
                        ),
                    )
                    .unwrap();
                barrier.release.notify_one();
            }
        );
        let op2 = op2.unwrap();
        assert_eq!(op2.phase, WorkflowDebugPhase::Applied { revision: 2 });
        assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
        let (one, two) = (op1.operation_id.clone(), op2.operation_id.clone());
        assert_eq!(
            round_history(&f),
            vec![
                (2, two.clone(), false),
                (3, one.clone(), false),
                (3, two.clone(), false),
                (4, one.clone(), true)
            ]
        );

        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&replay.node_run_id),
            ),
            (Some("new2"), Some("S1"), f.scope_of(&round_two.node_run_id))
        );
        f.finish_writer("T2");
        let next = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                next.prompt.as_deref(),
                next.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&next.node_run_id),
            ),
            (
                Some("new2"),
                Some("T2"),
                f.scope_of(&round_three.node_run_id)
            )
        );
        // Round 4 is still the first edit's archived slot; the second edit only took round 3.
        assert_eq!(
            round_history(&f),
            vec![
                (2, two.clone(), false),
                (3, one.clone(), false),
                (3, two, false),
                (4, one, true)
            ]
        );
    });
}

/// P2-1: a completed foreach's entrance edit (new input source) is accepted, and the process
/// stops while the independent `outside` runs. Boot fails the run for `outside` and applies the
/// edit to the failed run. Ordinary recovery must withdraw the old aggregate before the rebuilt
/// container dispatches its members.
#[test]
fn recovery_withdraws_the_old_aggregate_of_a_foreach_entrance_edit_applied_on_a_failed_run() {
    run_test(async {
        let agent = json!({"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},
            "executor":{"agentCli":"c","modelId":"m"},"prompt":"old"});
        let graph = json!({"nodes":[
            {"id":"start","data":{"kind":"start","inputVariables":[
                {"name":"items","valueType":"array[string]"},{"name":"others","valueType":"array[string]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                "iteratorSelector":["start","items"],"collectSelector":["member","output"],"errorStrategy":"fail"}}},
            {"id":"member","parentId":"iter","data":{"kind":"agent","agentConfig":agent}},
            {"id":"outside","data":{"kind":"agent","agentConfig":agent}}
        ],"edges":[
            {"source":"start","target":"iter"},{"source":"iter","target":"member"},
            {"source":"start","target":"outside"}
        ]})
        .to_string();
        let f = Fixture::with_inputs(
            Arc::new(NoSessions),
            &graph,
            [
                ("items".to_string(), json!(["a"])),
                ("others".to_string(), json!(["b"])),
            ]
            .into(),
        );
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("member").id,
                Some("A".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let owner = f.row("iter");
        assert_eq!(owner.status, WorkflowNodeStatus::Succeeded);
        let preview = f
            .coordinator
            .preview(
                serde_json::from_value(json!({
                    "runId": f.run_id.as_ref(), "nodeRunId": owner.id.as_ref(), "baseRevision": 0,
                    "patch": [{"op":"replace","path":"/iterationConfig/iteratorSelector","value":["start","others"]}]
                }))
                .unwrap(),
            )
            .unwrap();
        assert!(preview.impact.stop.is_empty());
        f.store.claim(&f.commit_request(&preview, "crash")).unwrap();
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
        assert_eq!(
            (
                f.run_status(),
                f.store.history(f.run_id.as_ref()).unwrap()[0]
                    .operation
                    .phase
                    .clone()
            ),
            (
                WorkflowRunStatus::Failed,
                WorkflowDebugPhase::Applied { revision: 1 }
            )
        );
        let values = |f: &Fixture| {
            let run = SqliteWorkflowRunRepository::new(f.pool.clone())
                .find_run(&f.run_id)
                .unwrap()
                .unwrap();
            serde_json::from_str::<ora_application::WorkflowRunPayload>(
                run.payload.as_deref().unwrap(),
            )
            .unwrap()
            .variable_pool
            .values
        };
        assert_eq!(values(&f).get("iter.output"), Some(&json!(["A"])));

        assert_eq!(
            engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(values(&f).get("iter.output"), None);
        let member = last_dispatch(&executor, "member");
        assert_eq!(
            (
                member.variable_pool.values.get("iter.item"),
                member.variable_pool.values.get("iter.output"),
            ),
            (Some(&json!("b")), None)
        );
        engine
            .complete_node(
                &f.run_id,
                &WorkflowNodeRunId::new(member.node_run_id),
                Some("B".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(values(&f).get("iter.output"), Some(&json!(["B"])));
    });
}

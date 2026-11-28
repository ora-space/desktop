//! A debug edit applied to a terminal run keeps the terminal projection, but its retired results
//! must leave the root pool before anything reads it again (invariant 3, D3 "J only reads b").
//! These cover the three paths that apply on a terminal run: a crash with out-of-scope work in
//! flight, and the reverse race for a root and for a while member edit (review 1: H1, H, H2).
use super::*;
use ora_application::WorkflowRunPayload;
use pretty_assertions::assert_eq;

/// start → c (yes: a → [a2 →] j, else: b → j) next to an independent agent s; j aggregates the
/// first available of `a.output`, `b.output`. `with_a2` puts a second agent on the yes branch.
fn reroute_graph(with_a2: bool) -> String {
    let agent = |prompt: &str| {
        json!({"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},
               "executor":{"agentCli":"c","modelId":"m"},"prompt":prompt})
    };
    let mut nodes = vec![
        json!({"id":"start","data":{"kind":"start","inputVariables":[{"name":"flag","valueType":"boolean"}]}}),
        json!({"id":"c","data":{"kind":"condition","cases":[{"id":"yes","logic":"and","conditions":[
            {"variableSelector":["start","flag"],"operator":"is","value":true}]}]}}),
        json!({"id":"a","data":{"kind":"agent","agentConfig":agent("a")}}),
        json!({"id":"b","data":{"kind":"agent","agentConfig":agent("b")}}),
        json!({"id":"s","data":{"kind":"agent","agentConfig":agent("s")}}),
        json!({"id":"j","data":{"kind":"aggregator","aggregatorConfig":{"variables":[["a","output"],["b","output"]]}}}),
    ];
    let mut edges = vec![
        json!({"source":"start","target":"c"}),
        json!({"source":"start","target":"s"}),
        json!({"source":"c","sourceHandle":"yes","target":"a"}),
        json!({"source":"c","sourceHandle":"else","target":"b"}),
        json!({"source":"b","target":"j"}),
    ];
    if with_a2 {
        nodes.push(json!({"id":"a2","data":{"kind":"agent","agentConfig":agent("a2")}}));
        edges.push(json!({"source":"a","target":"a2"}));
        edges.push(json!({"source":"a2","target":"j"}));
    } else {
        edges.push(json!({"source":"a","target":"j"}));
    }
    json!({"nodes":nodes,"edges":edges}).to_string()
}

/// The run's persisted payload.
fn payload(f: &Fixture) -> WorkflowRunPayload {
    let run = SqliteWorkflowRunRepository::new(f.pool.clone())
        .find_run(&f.run_id)
        .unwrap()
        .unwrap();
    serde_json::from_str(run.payload.as_deref().unwrap()).unwrap()
}

/// Root-pool keys among `keys` that currently hold a value.
fn present(payload: &WorkflowRunPayload, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter(|key| payload.variable_pool.values.contains_key(**key))
        .map(|key| key.to_string())
        .collect()
}

/// Flip the Condition `c` to its else branch.
fn reroute_preview(f: &Fixture) -> WorkflowDebugPreview {
    f.coordinator
        .preview(
            serde_json::from_value(json!({
                "runId": f.run_id.as_ref(), "nodeRunId": f.row("c").id.as_ref(), "baseRevision": 0,
                "patch": [{"op":"replace","path":"/cases/0/conditions/0/value","value":false}]
            }))
            .unwrap(),
        )
        .unwrap()
}

/// Complete every agent dispatched by `executor` from `skip` on with its own node id as output.
fn complete_dispatched_after(
    f: &Fixture,
    engine: &ConcreteEngine,
    executor: &RecordingExecutor,
    skip: usize,
) {
    for record in executor.records().into_iter().skip(skip) {
        engine
            .complete_node(
                &f.run_id,
                &WorkflowNodeRunId::new(record.node_run_id),
                Some(record.node_id.clone()),
                None,
                None,
                vec![],
            )
            .unwrap();
    }
}

type ConcreteEngine = ora_application::WorkflowRunEngine<
    SqliteWorkflowRunEngineRepository,
    UuidWorkflowNodeRunIdGenerator,
    SystemClock,
>;

/// H1: the crash lands after accepting a reroute while the independent s is in flight. Boot
/// fails the run for s and applies the edit on the failed run; recovery must withdraw the old
/// branch before b and s dispatch, so j reads b.
#[test]
fn recovery_after_a_crash_with_independent_work_withdraws_the_rerouted_branch() {
    run_test(async {
        let f = Fixture::with_inputs(
            Arc::new(NoSessions),
            &reroute_graph(false),
            [("flag".to_string(), json!(true))].into(),
        );
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("a").id,
                Some("a".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(f.row("j").status, WorkflowNodeStatus::Succeeded);
        let preview = reroute_preview(&f);
        assert!(preview.impact.stop.is_empty());
        f.store.claim(&f.commit_request(&preview, "crash")).unwrap();
        let before = payload(&f);

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
        // The failed run still shows what it ended with; only the withdrawal is parked.
        let parked = payload(&f);
        assert_eq!(
            (
                parked.variable_pool.values.clone(),
                parked.condition_decisions.clone(),
                parked.debug_parked_withdrawals.clone()
            ),
            (
                before.variable_pool.values.clone(),
                before.condition_decisions.clone(),
                ["a", "c", "j"].map(String::from).into()
            )
        );

        assert_eq!(
            engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        let resumed = payload(&f);
        assert_eq!(
            (
                present(&resumed, &["a.output", "j.output"]),
                resumed.condition_decisions.get("c").cloned(),
                resumed.debug_parked_withdrawals.is_empty()
            ),
            (vec![], Some("else".to_string()), true)
        );
        let mut nodes: Vec<_> = executor.records().into_iter().map(|r| r.node_id).collect();
        nodes.sort();
        assert_eq!(nodes, ["b", "s"]);
        assert!(
            executor
                .records()
                .iter()
                .all(|r| !r.variable_pool.values.contains_key("a.output"))
        );
        complete_dispatched_after(&f, &engine, &executor, 0);
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(payload(&f).variable_pool.values["j.output"], json!("b"));
    });
}

/// Probe H: flip c while a2 on the old branch stops, and fail the independent s meanwhile. The
/// revision applies on the failed run; returns the fixture and the payload the failure left.
async fn root_reverse_race() -> (Fixture, WorkflowRunPayload) {
    let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
    let f = Fixture::with_inputs(
        barrier.clone(),
        &reroute_graph(true),
        [("flag".to_string(), json!(true))].into(),
    );
    f.engine
        .complete_node(
            &f.run_id,
            &f.row("a").id,
            Some("a".into()),
            None,
            None,
            vec![],
        )
        .unwrap();
    for node in ["a2", "s"] {
        let record = last_dispatch(&f.executor, node);
        f.store
            .record_agent_dispatch(
                f.run_id.as_ref(),
                &record.node_run_id,
                &json!({"variablePool":record.variable_pool}),
            )
            .unwrap();
    }
    let a2 = last_dispatch(&f.executor, "a2");
    f.bind_agent(&WorkflowNodeRunId::new(&a2.node_run_id));
    let preview = reroute_preview(&f);
    assert_eq!(preview.impact.stop, vec![a2.node_run_id.clone()]);
    let (op, failed) = tokio::join!(
        f.coordinator.commit(f.commit_request(&preview, "race")),
        async {
            barrier.entered.notified().await;
            f.engine
                .fail_node(
                    &f.run_id,
                    &f.row("s").id,
                    ora_application::NodeFailure::new(
                        ora_application::NodeFailureKind::Session,
                        "s failed",
                    ),
                )
                .unwrap();
            let failed = payload(&f);
            barrier.release.notify_one();
            failed
        }
    );
    assert_eq!(
        op.unwrap().phase,
        WorkflowDebugPhase::Applied { revision: 1 }
    );
    assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
    (f, failed)
}

/// Pre-existing (probe H): a root reroute whose stop races an independent failure applies on
/// the failed run. Ordinary recovery withdraws the retired branch before rerunning c, so j
/// reads b and not the retired a.
#[test]
fn ordinary_recovery_after_a_root_reverse_race_withdraws_the_retired_branch() {
    run_test(async {
        let (f, failed) = root_reverse_race().await;
        let parked = payload(&f);
        assert_eq!(
            (
                parked.variable_pool.values.clone(),
                parked.debug_parked_withdrawals.clone()
            ),
            (
                failed.variable_pool.values.clone(),
                ["a", "a2", "c"].map(String::from).into()
            )
        );
        let skip = f.executor.records().len();
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(present(&payload(&f), &["a.output"]), Vec::<String>::new());
        let mut nodes: Vec<_> = f
            .executor
            .records()
            .into_iter()
            .skip(skip)
            .map(|r| r.node_id)
            .collect();
        nodes.sort();
        assert_eq!(nodes, ["b", "s"]);
        complete_dispatched_after(&f, &f.engine, &f.executor, skip);
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(payload(&f).variable_pool.values["j.output"], json!("b"));
        assert!(payload(&f).debug_parked_withdrawals.is_empty());
    });
}

/// H2: a while member edit loses the reverse race after its Loop already finished and fed a
/// downstream branch. The parked round reopen carries the root withdrawal: on recovery the
/// Loop's old export, the old route and the old branch output leave the pool before the
/// re-dispatched `outside` or the replayed writer read it, and j finally reads b.
#[test]
fn ordinary_recovery_of_a_parked_round_withdraws_the_loop_downstream() {
    run_test(async {
        let agent = |prompt: &str| {
            json!({"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},
                   "executor":{"agentCli":"c","modelId":"m"},"prompt":prompt})
        };
        let graph = json!({"schemaVersion":2,"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"loop","data":{"kind":"loop","loopConfig":{
                "maxIterations":3,
                "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":"seed"},"feedback":["writer","output"]}],
                "until":{"logic":"or","conditions":[
                    {"variableSelector":["writer","output"],"operator":"equals","value":"done"},
                    {"variableSelector":["writer","output"],"operator":"equals","value":"done2"}]},
                "outputs":[{"name":"result","variableSelector":["writer","output"]}]}}},
            {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
            {"id":"writer","parentId":"loop","data":{"kind":"agent","containerId":"loop","agentConfig":agent("revise")}},
            {"id":"c","data":{"kind":"condition","cases":[{"id":"yes","logic":"and","conditions":[
                {"variableSelector":["loop","result"],"operator":"equals","value":"done"}]}]}},
            {"id":"a","data":{"kind":"agent","agentConfig":agent("a")}},
            {"id":"a2","data":{"kind":"agent","agentConfig":agent("a2")}},
            {"id":"b","data":{"kind":"agent","agentConfig":agent("b")}},
            {"id":"outside","data":{"kind":"agent","agentConfig":agent("other")}},
            {"id":"j","data":{"kind":"aggregator","aggregatorConfig":{"variables":[["a","output"],["b","output"]]}}}
        ],"edges":[
            {"source":"start","target":"loop"},{"source":"entry","target":"writer"},
            {"source":"loop","target":"c"},{"source":"c","sourceHandle":"yes","target":"a"},
            {"source":"a","target":"a2"},{"source":"c","sourceHandle":"else","target":"b"},
            {"source":"a2","target":"j"},{"source":"b","target":"j"},
            {"source":"start","target":"outside"}]})
        .to_string();
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &graph);
        let selected = f.observe_live_writer();
        f.finish_writer("done");
        f.engine
            .complete_node(
                &f.run_id,
                &WorkflowNodeRunId::new(last_dispatch(&f.executor, "a").node_run_id),
                Some("a".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let a2 = last_dispatch(&f.executor, "a2");
        f.store
            .record_agent_dispatch(
                f.run_id.as_ref(),
                &a2.node_run_id,
                &json!({"variablePool":a2.variable_pool}),
            )
            .unwrap();
        f.bind_agent(&WorkflowNodeRunId::new(&a2.node_run_id));
        let preview = f.prompt_preview(&selected.node_run_id, 0, "new");
        assert_eq!(
            (preview.target.round, preview.impact.stop.clone()),
            (Some(1), vec![a2.node_run_id.clone()])
        );
        let (op, failed) = tokio::join!(
            f.coordinator.commit(f.commit_request(&preview, "race")),
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
                let failed = payload(&f);
                barrier.release.notify_one();
                failed
            }
        );
        assert_eq!(
            op.unwrap().phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
        // The failed run keeps its visible pool byte for byte; the withdrawal rides with the
        // parked round reopen on the Loop owner.
        assert_eq!(payload(&f), failed);
        let stale = ["loop.result", "a.output"];
        assert_eq!(present(&failed, &stale), stale.map(String::from));

        let skip = f.executor.records().len();
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        let resumed = payload(&f);
        assert_eq!(
            (
                present(&resumed, &stale),
                resumed.condition_decisions.get("c").cloned()
            ),
            (vec![], None)
        );
        let owner_payload: serde_json::Value =
            serde_json::from_str(f.row("loop").payload.as_deref().unwrap_or("{}")).unwrap();
        assert_eq!(owner_payload.get("debugWhileReplays"), None);
        // Nothing dispatched after recovery reads a retired value.
        let redispatched = f
            .executor
            .records()
            .into_iter()
            .skip(skip)
            .collect::<Vec<_>>();
        let mut nodes: Vec<_> = redispatched.iter().map(|r| r.node_id.clone()).collect();
        nodes.sort();
        assert_eq!(nodes, ["outside", "writer"]);
        for record in &redispatched {
            assert_eq!(
                (
                    record.node_id.as_str(),
                    present_in(&record.variable_pool, &stale)
                ),
                (record.node_id.as_str(), vec![])
            );
        }
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str()
            ),
            (Some("new"), Some("seed"))
        );
        let rest = f.executor.records().len();
        f.finish_writer("done2");
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
        complete_dispatched_after(&f, &f.engine, &f.executor, rest);
        let after: Vec<_> = f
            .executor
            .records()
            .into_iter()
            .skip(skip)
            .map(|r| r.node_id)
            .collect();
        assert!(!after.iter().any(|node| node == "a" || node == "a2"));
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(
            f.row("loop").output.as_deref(),
            Some(r#"{"result":"done2"}"#)
        );
        assert_eq!(payload(&f).variable_pool.values["j.output"], json!("b"));
    });
}

/// Keys among `keys` present in a dispatched variable pool.
fn present_in(pool: &ora_application::WorkflowVariablePool, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter(|key| pool.values.contains_key(**key))
        .map(|key| key.to_string())
        .collect()
}

/// The same root reverse race recovered by debugging the failed sibling: reopening the run for
/// that edit performs the parked withdrawal first, so neither the re-dispatched s nor the run's
/// current variables keep the retired branch. The earlier edit's positions stay unauthorized
/// for this terminal edit (#830 `self_failed_stop_then_terminal_edit_uses_fresh_baseline`).
#[test]
fn debugging_the_failed_sibling_after_a_root_reverse_race_withdraws_the_retired_branch() {
    run_test(async {
        let (f, _) = root_reverse_race().await;
        let s = f.row("s");
        assert_eq!(present(&payload(&f), &["a.output"]), vec!["a.output"]);
        let sibling = f.prompt_preview(s.id.as_ref(), 1, "fixed");
        assert_eq!(sibling.impact.schedule, ["s"]);
        let skip = f.executor.records().len();
        let second = f
            .coordinator
            .commit(f.commit_request(&sibling, "sibling"))
            .await
            .unwrap();
        assert_eq!(second.phase, WorkflowDebugPhase::Applied { revision: 2 });
        let reopened = payload(&f);
        assert_eq!(
            (
                present(&reopened, &["a.output"]),
                reopened.condition_decisions.get("c").cloned(),
                reopened.debug_parked_withdrawals.is_empty()
            ),
            (vec![], None, true)
        );
        let redispatched: Vec<_> = f.executor.records().into_iter().skip(skip).collect();
        assert_eq!(
            redispatched
                .iter()
                .map(|r| (
                    r.node_id.as_str(),
                    r.variable_pool.values.contains_key("a.output")
                ))
                .collect::<Vec<_>>(),
            [("s", false)]
        );
        complete_dispatched_after(&f, &f.engine, &f.executor, skip);
        // Required positions of the earlier edit have no result, so the run returns to its
        // saved outcome (D1) and its current variables no longer show the retired branch.
        let settled = payload(&f);
        assert_eq!(
            (f.run_status(), present(&settled, &["a.output", "j.output"])),
            (WorkflowRunStatus::Failed, vec![])
        );
    });
}

/// A whole-run restart builds a new root. Withdrawals parked for the old root are moot and must
/// not follow it, or a later recovery would retire the new root's results.
#[test]
fn a_whole_run_restart_forgets_parked_withdrawals() {
    run_test(async {
        let (f, _) = root_reverse_race().await;
        assert!(!payload(&f).debug_parked_withdrawals.is_empty());
        assert_eq!(
            f.engine.restart(&f.run_id).unwrap(),
            ora_application::RestartWorkflowRunResult::Restarted
        );
        assert_eq!(
            (
                f.run_status(),
                payload(&f).debug_parked_withdrawals.is_empty()
            ),
            (WorkflowRunStatus::Running, true)
        );
    });
}

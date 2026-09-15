//! End-to-end verification of the iteration composite runtime against real SQLite.
//!
//! Every test drives the production engine over a frozen graph and asserts on persisted rows,
//! the run payload's ledger and variable pool, and the run state machine — the evidence
//! obligations of `test-cases/desktop/core/workflow/iteration-node.md`.

use super::test_fixture::{NoopExecutor, SeqGen, bootstrap, seeded_pending_run};
use ora_application::{
    WorkflowRunEngine, WorkflowRunEngineRepository, WorkflowRunPayload, WorkflowVariablePool,
};
use ora_contracts::WorkflowRunLocale;
use ora_db::SqliteWorkflowRunEngineRepository;
use ora_domain::{WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use serde_json::{Value, json};

/// Builds an iteration graph over a Start `prs` array source.
///
/// The region is entered at `entry` (an agent named `fix` or a condition named `gate`) and
/// collects `fix.output` per round; the node exits to a terminal output node.
fn iteration_graph(body: &str, error_strategy: &str, max_iterations: u32) -> String {
    let (nodes, edges) = match body {
        "condition" => (
            r#"[{"id":"gate","parentId":"iter","data":{"kind":"condition","cases":[
                {"id":"fix-it","logic":"and","conditions":[
                    {"variableSelector":["iter","item","id"],"operator":"equals","value":2}
                ]}
            ]}},
            {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{
                "executor":{"agentCli":"open_code","modelId":"m"},"prompt":"fix {{#iter.item#}}"
            }}}]"#,
            r#"[{"source":"iter","target":"gate"},
                {"source":"gate","sourceHandle":"fix-it","target":"fix"}]"#,
        ),
        _ => (
            r#"[{"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{
                "executor":{"agentCli":"open_code","modelId":"m"},"prompt":"fix {{#iter.item#}}"
            }}}]"#,
            r#"[{"source":"iter","target":"fix"}]"#,
        ),
    };
    let mut all_nodes: Vec<Value> = json!([
        { "id": "start", "data": { "kind": "start", "inputVariables": [
            { "name": "prs", "valueType": "array[object]" }
        ] } },
        { "id": "iter", "data": { "kind": "iteration", "iterationConfig": {
            "iteratorSelector": ["start", "prs"],
            "collectSelector": ["fix", "output"],
            "errorStrategy": error_strategy,
            "maxIterations": max_iterations
        } } },
        { "id": "out", "data": { "kind": "output", "outputs": [
            { "name": "collected", "variableSelector": ["iter", "output"] },
            { "name": "failedCount", "variableSelector": ["iter", "failed_count"] }
        ] } }
    ])
    .as_array()
    .unwrap()
    .clone();
    all_nodes.extend(serde_json::from_str::<Vec<Value>>(nodes).unwrap());
    let mut all_edges: Vec<Value> = json!([
        { "source": "start", "target": "iter" },
        { "source": "iter", "target": "out" }
    ])
    .as_array()
    .unwrap()
    .clone();
    all_edges.extend(serde_json::from_str::<Vec<Value>>(edges).unwrap());
    json!({ "nodes": all_nodes, "edges": all_edges }).to_string()
}

/// Seeds a pending run whose Start variable `prs` holds `items`, then returns the run id.
fn seeded_iteration_run(
    temp: &tempfile::TempDir,
    pool: &ora_db::RepositoryPool,
    graph: &str,
    items: Value,
) -> WorkflowRunId {
    let run_id = seeded_pending_run(temp, pool, graph);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let mut variables = std::collections::BTreeMap::new();
    variables.insert("prs".to_string(), items);
    repository
        .update_run_input(&run_id, Some("kickoff".to_string()), variables, 35)
        .unwrap();
    run_id
}

/// The persisted run payload of one run.
fn run_payload(
    repository: &SqliteWorkflowRunEngineRepository,
    run_id: &WorkflowRunId,
) -> WorkflowRunPayload {
    let context = repository
        .find_execution_context(run_id)
        .unwrap()
        .expect("run context");
    serde_json::from_str(context.run.payload.as_deref().expect("run payload")).unwrap()
}

/// The `Running` region rows of the given node, in round order.
fn running_region_rows(
    repository: &SqliteWorkflowRunEngineRepository,
    run_id: &WorkflowRunId,
    node_id: &str,
) -> Vec<ora_domain::WorkflowNodeRun> {
    repository
        .list_node_runs(run_id)
        .unwrap()
        .into_iter()
        .filter(|row| row.node_id == node_id && row.status == WorkflowNodeStatus::Running)
        .collect()
}

/// Drives every running region agent to completion with a per-round output.
fn complete_running_rounds(
    engine: &WorkflowRunEngine<
        SqliteWorkflowRunEngineRepository,
        SeqGen,
        crate::workflow::run::test_fixture::ClockAt,
    >,
    repository: &SqliteWorkflowRunEngineRepository,
    run_id: &WorkflowRunId,
    node_id: &str,
    output_of: impl Fn(u32) -> String,
) {
    loop {
        let rows = running_region_rows(repository, run_id, node_id);
        if rows.is_empty() {
            return;
        }
        for row in rows {
            let output = output_of(row.iteration.unwrap_or_default());
            engine
                .complete_node(run_id, &row.id, Some(output), None, None, Vec::new())
                .unwrap();
        }
    }
}

/// Serial foreach: every round runs in order, each round binds its own item and index, node
/// outputs overwrite per round, and the node completes with the full ledger projection that
/// the terminal output node consumes (iteration-node.md: Loop Iterations Are Persisted Facts,
/// continue Strategy Absorbs Failed Rounds' projection shape).
#[test]
fn serial_foreach_executes_every_round_and_exposes_the_ledger() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "fail", 10);
        let run_id = seeded_iteration_run(
            &temp,
            &pool,
            &graph,
            json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]),
        );
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        complete_running_rounds(&engine, &repository, &run_id, "fix", |round| {
            format!("fixed round {round}")
        });

        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Succeeded);
        // The terminal output consumed the exposed variables.
        assert_eq!(
            context.run.output.as_deref(),
            Some(
                r#"{"collected":["fixed round 0","fixed round 1","fixed round 2"],"failedCount":0}"#
            )
        );

        // One row per round, each carrying its round index; the composite runtime drives its
        // region without ever creating or binding a session of its own.
        let fix_rows: Vec<_> = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .filter(|row| row.node_id == "fix")
            .collect();
        assert_eq!(fix_rows.len(), 3);
        assert_eq!(
            fix_rows.iter().map(|row| row.iteration).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2)]
        );
        for row in repository.list_node_runs(&run_id).unwrap() {
            assert_eq!(row.session_id, None, "row {} must hold no session", row.id);
        }

        let payload = run_payload(&repository, &run_id);
        // Per-round variables: the latest round's bindings remain after completion.
        assert_eq!(
            payload.variable_pool.values.get("iter.item").unwrap(),
            &json!({ "id": 3 })
        );
        assert_eq!(
            payload.variable_pool.values.get("iter.index").unwrap(),
            &json!(2)
        );
        // The overwrite semantics: fix.output resolves to the latest round's value.
        assert_eq!(
            payload.variable_pool.values.get("fix.output").unwrap(),
            &json!("fixed round 2")
        );
        // The exposed projection is ledger-derived, not a text pool write.
        assert_eq!(
            payload.variable_pool.values.get("iter.output").unwrap(),
            &json!(["fixed round 0", "fixed round 1", "fixed round 2"])
        );
        assert_eq!(
            payload
                .variable_pool
                .values
                .get("iter.failed_count")
                .unwrap(),
            &json!(0)
        );
        let entries = payload
            .variable_pool
            .values
            .get("iter.entries")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["item"], json!({ "id": 1 }));
        assert_eq!(entries[0]["output"], json!("fixed round 0"));
        assert_eq!(entries[0]["status"], json!("succeeded"));
    });
}

/// An empty source is a legal input: the node succeeds immediately, no region row exists, and
/// the three exposed variables are empty (iteration-node.md: Empty Iterator Array...).
#[test]
fn an_empty_iterator_source_completes_with_empty_outputs() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "fail", 10);
        let run_id = seeded_iteration_run(&temp, &pool, &graph, json!([]));
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();

        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Succeeded);
        assert!(
            repository
                .list_node_runs(&run_id)
                .unwrap()
                .into_iter()
                .all(|row| row.iteration.is_none())
        );
        let payload = run_payload(&repository, &run_id);
        assert_eq!(
            payload.variable_pool.values.get("iter.output").unwrap(),
            &json!([])
        );
        assert_eq!(
            payload.variable_pool.values.get("iter.entries").unwrap(),
            &json!([])
        );
        assert_eq!(
            payload
                .variable_pool
                .values
                .get("iter.failed_count")
                .unwrap(),
            &json!(0)
        );
    });
}

/// A source longer than maxIterations fails at the startup boundary — even under `continue`,
/// because the node's own failures never absorb — with both numbers in the error and no round
/// executed (iteration-node.md: Iteration Fails At The Startup Boundary...).
#[test]
fn a_source_exceeding_the_ceiling_fails_at_the_startup_boundary() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "continue", 2);
        let run_id = seeded_iteration_run(
            &temp,
            &pool,
            &graph,
            json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]),
        );
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();

        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Failed);
        let error = context.run.error.expect("run error").clone();
        assert!(error.contains("3 elements"), "{error}");
        assert!(error.contains("maxIterations 2"), "{error}");
        // No round ever executed.
        assert!(
            repository
                .list_node_runs(&run_id)
                .unwrap()
                .into_iter()
                .all(|row| row.iteration.is_none())
        );
    });
}

/// `fail` strategy: the first failed round fails the node and the run; later rounds never
/// start, and the already-committed round history stays (iteration-node.md: fail Strategy...).
#[test]
fn fail_strategy_stops_at_the_first_failed_round() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "fail", 10);
        let run_id = seeded_iteration_run(
            &temp,
            &pool,
            &graph,
            json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]),
        );
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();

        // Round 0 succeeds; round 1's agent fails through the callback path.
        let rows = running_region_rows(&repository, &run_id, "fix");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].iteration, Some(0));
        engine
            .complete_node(
                &run_id,
                &rows[0].id,
                Some("ok".to_string()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let rows = running_region_rows(&repository, &run_id, "fix");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].iteration, Some(1));
        engine
            .fail_node(&run_id, &rows[0].id, "agent exploded".to_string(), None)
            .unwrap();

        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Failed);
        let iter_row = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .find(|row| row.node_id == "iter")
            .unwrap();
        assert_eq!(iter_row.status, WorkflowNodeStatus::Failed);
        // Rounds 2 never started; round 0's committed history survives.
        let fix_rounds: Vec<_> = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .filter(|row| row.node_id == "fix")
            .map(|row| (row.iteration, row.status))
            .collect();
        assert_eq!(
            fix_rounds,
            vec![
                (Some(0), WorkflowNodeStatus::Succeeded),
                (Some(1), WorkflowNodeStatus::Failed),
            ]
        );
    });
}

/// `continue` strategy: failed rounds are absorbed into the ledger with their error attribution,
/// the remaining rounds still run, and the node completes with the filtered projection
/// (iteration-node.md: continue Strategy Absorbs Failed Rounds Into The Ledger).
#[test]
fn continue_strategy_absorbs_failed_rounds_into_the_ledger() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "continue", 10);
        let run_id = seeded_iteration_run(
            &temp,
            &pool,
            &graph,
            json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]),
        );
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();

        // Round 0 succeeds; round 1 fails and is absorbed; round 2 succeeds.
        let rows = running_region_rows(&repository, &run_id, "fix");
        engine
            .complete_node(
                &run_id,
                &rows[0].id,
                Some("first".to_string()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let rows = running_region_rows(&repository, &run_id, "fix");
        assert_eq!(rows[0].iteration, Some(1));
        engine
            .fail_node(&run_id, &rows[0].id, "agent exploded".to_string(), None)
            .unwrap();
        complete_running_rounds(&engine, &repository, &run_id, "fix", |_| {
            "third".to_string()
        });

        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Succeeded);
        let iter_row = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .find(|row| row.node_id == "iter")
            .unwrap();
        assert_eq!(iter_row.status, WorkflowNodeStatus::Succeeded);

        let payload = run_payload(&repository, &run_id);
        // `output` keeps only succeeded rounds, in input order.
        assert_eq!(
            payload.variable_pool.values.get("iter.output").unwrap(),
            &json!(["first", "third"])
        );
        assert_eq!(
            payload
                .variable_pool
                .values
                .get("iter.failed_count")
                .unwrap(),
            &json!(1)
        );
        // `entries` stays aligned with the input array and keeps the failure attribution.
        let entries = payload
            .variable_pool
            .values
            .get("iter.entries")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["output"], json!("first"));
        assert_eq!(entries[1]["status"], json!("failed"));
        assert_eq!(entries[1]["error"], json!("agent exploded"));
        assert_eq!(entries[1]["output"], json!(null));
        assert_eq!(entries[2]["output"], json!("third"));
    });
}

/// Per-round Condition decisions: the same region condition answers differently per round,
/// each round's branch activation follows its own decision, a bypassed collect target settles
/// as a failed round, and the decisions persist per round (iteration-node.md:
/// Per-Round Condition Decisions Are Scoped To Their Round; Round Settlement Requires...).
#[test]
fn per_round_condition_decisions_settle_bypassed_rounds_as_failed() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        // The gate fixes only item id 2; rounds with id 1 and 3 bypass the collect target.
        let graph = iteration_graph("condition", "continue", 10);
        let run_id = seeded_iteration_run(
            &temp,
            &pool,
            &graph,
            json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]),
        );
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        complete_running_rounds(&engine, &repository, &run_id, "fix", |round| {
            format!("fixed round {round}")
        });

        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Succeeded);

        let payload = run_payload(&repository, &run_id);
        // The per-round decisions persist keyed by condition and round.
        assert_eq!(
            payload.iteration_condition_decisions,
            std::collections::BTreeMap::from([
                ("gate#0".to_string(), "else".to_string()),
                ("gate#1".to_string(), "fix-it".to_string()),
                ("gate#2".to_string(), "else".to_string()),
            ])
        );
        // Only the gated round collected; bypassed rounds settled as failed with the
        // structural-check reason, never with a stale pool value.
        assert_eq!(
            payload.variable_pool.values.get("iter.output").unwrap(),
            &json!(["fixed round 1"])
        );
        let entries = payload
            .variable_pool
            .values
            .get("iter.entries")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0]["error"],
            json!("collect target did not run this round")
        );
        assert_eq!(entries[1]["output"], json!("fixed round 1"));
        assert_eq!(
            entries[2]["error"],
            json!("collect target did not run this round")
        );
        assert_eq!(
            payload
                .variable_pool
                .values
                .get("iter.failed_count")
                .unwrap(),
            &json!(2)
        );
    });
}

/// Region rows never enter the outer `current_nodes` anchor and the outer scheduler never
/// dispatches them directly: while a round is in flight, the anchor holds the iteration node
/// alone (region-and-loop.md: Region Interior Nodes Never Enter The Outer Ready Set).
#[test]
fn region_rows_never_enter_the_outer_current_nodes_anchor() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "fail", 10);
        let run_id = seeded_iteration_run(&temp, &pool, &graph, json!([{ "id": 1 }, { "id": 2 }]));
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();

        // Round 0's agent is in flight: the anchor names the iteration node only.
        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        let state: serde_json::Value =
            serde_json::from_str(context.run.state.as_deref().unwrap()).unwrap();
        assert_eq!(state["current_nodes"], json!(["iter"]));

        complete_running_rounds(&engine, &repository, &run_id, "fix", |round| {
            format!("fixed round {round}")
        });
        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Succeeded);
        let state: serde_json::Value =
            serde_json::from_str(context.run.state.as_deref().unwrap()).unwrap();
        assert_eq!(state["current_nodes"], json!([]));
    });
}

/// Restart resets the runtime state: the ledger and per-round decisions clear while Start
/// deployment values survive, so a rerun executes every round again from scratch.
#[test]
fn restart_clears_the_ledger_and_per_round_decisions() {
    crate::workflow::run::test_fixture::run_test(async {
        let (temp, pool) = bootstrap();
        let graph = iteration_graph("agent", "fail", 10);
        let run_id = seeded_iteration_run(&temp, &pool, &graph, json!([{ "id": 1 }, { "id": 2 }]));
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            NoopExecutor,
            SeqGen::default(),
            crate::workflow::run::test_fixture::ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        complete_running_rounds(&engine, &repository, &run_id, "fix", |round| {
            format!("fixed round {round}")
        });
        let payload = run_payload(&repository, &run_id);
        assert!(!payload.iteration_ledger.is_empty());

        engine.restart(&run_id).unwrap();
        let context = repository
            .find_execution_context(&run_id)
            .unwrap()
            .expect("run context");
        assert_eq!(context.run.status, WorkflowRunStatus::Running);
        let payload = run_payload(&repository, &run_id);
        assert!(payload.iteration_ledger.is_empty());
        assert!(payload.iteration_condition_decisions.is_empty());
        // Start deployment values survive the restart for the fresh execution.
        assert_eq!(
            payload.variable_pool.values.get("start.prs").unwrap(),
            &json!([{ "id": 1 }, { "id": 2 }])
        );
    });
}

/// A legacy payload without the iteration fields parses unchanged: old runs (including HITL
/// runs parked for days) stay readable after the upgrade (iteration-node.md:
/// Per-Round Condition Decisions..., old payload parse compatibility).
#[test]
fn legacy_payloads_without_iteration_fields_parse() {
    let legacy = r#"{
        "locale":"zh-CN",
        "skillMaterialization":{"bindings":[]},
        "variablePool":{"revision":0,"catalog":{},"values":{}},
        "conditionDecisions":{"c":"case-1"}
    }"#;
    let payload: WorkflowRunPayload = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        payload.condition_decisions.get("c").map(String::as_str),
        Some("case-1")
    );
    assert!(payload.iteration_ledger.is_empty());
    assert!(payload.iteration_condition_decisions.is_empty());
    assert_eq!(
        payload.resolved_condition_decisions(),
        std::collections::BTreeMap::from([("c".to_string(), "case-1".to_string())])
    );
}

/// The declared types of the three exposed variables are fixed by the collect target's
/// declaration and never depend on the error strategy (iteration-node.md:
/// Exposed Iteration Variables Keep Fixed Types Across Error Strategies).
#[test]
fn exposed_variable_types_stay_fixed_across_error_strategies() {
    let catalog = |strategy: &str| {
        let graph =
            ora_application::WorkflowGraph::parse(&iteration_graph("agent", strategy, 10)).unwrap();
        let pool = WorkflowVariablePool::from_graph(&graph);
        pool.catalog
    };
    for strategy in ["fail", "continue"] {
        let catalog = catalog(strategy);
        assert_eq!(
            catalog.get("iter.output").map(|d| d.value_type.as_str()),
            Some("array[string]"),
            "output element type follows the collect target for {strategy}"
        );
        assert_eq!(
            catalog.get("iter.entries").map(|d| d.value_type.as_str()),
            Some("array[object]")
        );
        assert_eq!(
            catalog
                .get("iter.failed_count")
                .map(|d| d.value_type.as_str()),
            Some("number")
        );
        assert_eq!(
            catalog.get("iter.item").map(|d| d.value_type.as_str()),
            Some("object")
        );
        assert_eq!(
            catalog.get("iter.index").map(|d| d.value_type.as_str()),
            Some("number")
        );
    }
}

/// Sanity for the fixture: a graph with locale and pool wiring still round-trips through the
/// deployment payload shape used above.
#[test]
fn iteration_fixture_payload_round_trips() {
    let graph =
        ora_application::WorkflowGraph::parse(&iteration_graph("agent", "fail", 10)).unwrap();
    let mut pool = WorkflowVariablePool::from_graph(&graph);
    pool.set("start.prs", "start", json!([{ "id": 1 }]))
        .unwrap();
    let payload = WorkflowRunPayload::with_variable_pool(
        WorkflowRunLocale::EnUs,
        Default::default(),
        Some("start".to_string()),
        pool,
    );
    let encoded = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        serde_json::from_str::<WorkflowRunPayload>(&encoded).unwrap(),
        payload
    );
}

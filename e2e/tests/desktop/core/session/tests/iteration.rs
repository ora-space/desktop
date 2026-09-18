//! End-to-end coverage for iteration dispatch, round bindings, and swift failures.

use super::{
    current_thread_runtime, install_fake_opencode_plugin, open_ready_backend, seed_workspace,
};
use crate::setup::DesktopTestSetup;
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Keeps the production graph fixed while varying the prompt and iteration failure policy.
fn graph(prompt: &str, strategy: &str, ceiling: u32) -> Value {
    json!({"nodes": [
        {"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","inputVariables":[
            {"name":"items","valueType":"array[string]"},
            {"name":"unset","valueType":"string"}
        ]}},
        {"id":"iter","type":"workflow","position":{"x":360,"y":0},"initialWidth":760,"initialHeight":420,"data":{"kind":"iteration","iterationConfig":{
            "iteratorSelector":["start","items"],"collectSelector":["body","output"],
            "errorStrategy":strategy,"maxIterations":ceiling
        }}},
        {"id":"body","type":"workflow","parentId":"iter","position":{"x":420,"y":160},"data":{"kind":"agent","agentConfig":{
            "executor":{"agentCli":"official/ora-space.opencode","modelId":"anthropic/claude-sonnet-4"},
            "prompt":prompt,"interactive":false
        }}},
        {"id":"out","type":"workflow","position":{"x":1240,"y":0},"data":{"kind":"output","outputs":[
            {"name":"collected","variableSelector":["iter","output"]}
        ]}}
    ],"edges":[
        {"source":"start","target":"iter"},
        {"source":"iter","sourceHandle":"iteration-entry","target":"body"},
        {"source":"iter","target":"out"}
    ]})
}

/// Builds the editor-authored multi-node region shape: entry → Condition branch → Agent.
fn multi_node_region_graph() -> Value {
    let mut graph = graph("item={{#iter.item#}} index={{#iter.index#}}", "fail", 10);
    let nodes = graph_array(&mut graph, "nodes");
    nodes.push(json!({
        "id":"gate","type":"workflow","parentId":"iter","position":{"x":96,"y":160},
        "data":{"kind":"condition","cases":[
            {"id":"non-empty","logic":"and","conditions":[
                {"variableSelector":["iter","item"],"operator":"not_empty"}
            ]}
        ]}
    }));
    graph["edges"][1]["target"] = json!("gate");
    graph_array(&mut graph, "edges").push(json!({
        "source":"gate","sourceHandle":"non-empty","target":"body"
    }));
    graph
}

/// Adds a condition whose comparison reads a declared but unassigned Start value.
fn failing_condition_graph(strategy: &str) -> Value {
    let mut graph = graph("hello", strategy, 10);
    let nodes = graph_array(&mut graph, "nodes");
    nodes.push(json!({
        "id":"gate","parentId":"iter","data":{"kind":"condition","cases":[
            {"id":"yes","logic":"and","conditions":[
                {"variableSelector":["start","unset"],"operator":"equals","value":"x"}
            ]}
        ]}
    }));
    graph["edges"][1]["target"] = json!("gate");
    graph_array(&mut graph, "edges").push(json!({
        "source":"gate","sourceHandle":"yes","target":"body"
    }));
    graph
}

/// Returns a required graph array while preserving a useful fixture failure message.
fn graph_array<'a>(graph: &'a mut Value, field: &str) -> &'a mut Vec<Value> {
    let Some(array) = graph.get_mut(field).and_then(Value::as_array_mut) else {
        panic!("iteration fixture graph field `{field}` must be an array");
    };
    array
}

/// Runs one graph through the real Backend and fake ACP plugin, then checks its terminal state.
fn run_case(
    graph: Value,
    items: Value,
    expected: WorkflowRunStatus,
    sessions: usize,
    expected_output_fragment: Option<&str>,
    expected_collected_items: Option<usize>,
) -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
            let setup = DesktopTestSetup::new()?;
            let package = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
            let backend = open_ready_backend(&setup)?;
            let workspace_id = seed_workspace(&setup, &backend)?;
            let workflow = backend
                .workflows()
                .create(CreateWorkflowRequest {
                    name: "Iteration E2E".into(),
                    graph: Some(graph.to_string()),
                })?
                .workflow;
            backend.workflows().publish(PublishWorkflowRequest {
                workflow_id: workflow.id.clone(),
                version: Some("v1".into()),
            })?;
            let run = backend
                .workflow_runs()
                .create(CreateWorkflowRunRequest {
                    inject_last_failure: None,
                    workspace_id,
                    workflow_id: workflow.id,
                    locale: WorkflowRunLocale::EnUs,
                    snapshot_id: None,
                    kickoff_input: None,
                    name: None,
                })?
                .run;
            backend
                .workflow_runs()
                .update_input(UpdateWorkflowRunInputRequest {
                    run_id: run.id.clone(),
                    input: None,
                    variables: BTreeMap::from([("items".into(), items)]),
                })?;
            backend.workflow_runs().start(StartWorkflowRunRequest {
                run_id: run.id.clone(),
            })?;

            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            let mut observed = loop {
                let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
                    run_id: run.id.clone(),
                })?;
                if matches!(
                    detail.run.status,
                    WorkflowRunStatus::Succeeded | WorkflowRunStatus::Failed
                ) || tokio::time::Instant::now() >= deadline
                {
                    break detail;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            };
            if observed.run.status == WorkflowRunStatus::Running {
                let cancelled = backend
                    .workflow_runs()
                    .cancel(CancelWorkflowRunRequest {
                        run_id: run.id.clone(),
                    })
                    .await?;
                observed = backend.workflow_runs().get(GetWorkflowRunRequest {
                    run_id: run.id.clone(),
                })?;
                assert_eq!(cancelled.run.status, WorkflowRunStatus::Cancelled);
            }
            let journal =
                std::fs::read_to_string(package.join("acp_calls.txt")).unwrap_or_default();
            let session_count = observed
                .nodes
                .iter()
                .filter(|node| node.session_id.is_some())
                .count();
            assert_eq!(observed.run.status, expected, "ACP journal: {journal}");
            assert_eq!(session_count, sessions, "ACP journal: {journal}");
            if let Some(fragment) = expected_output_fragment {
                assert!(
                    observed.nodes.iter().any(|node| {
                        node.output
                            .as_deref()
                            .is_some_and(|output| output.contains(fragment))
                    }),
                    "no node output contained {fragment:?}; ACP journal: {journal}",
                );
            }
            if let Some(expected_items) = expected_collected_items {
                let run_output = observed
                    .run
                    .output
                    .as_deref()
                    .ok_or("successful iteration run must expose output")?;
                let collected = serde_json::from_str::<Value>(run_output)?["collected"]
                    .as_array()
                    .ok_or("iteration output must contain a collected array")?
                    .len();
                assert_eq!(collected, expected_items);
            }
            Ok(())
        })
    })
}

/// Every committed round gets a real session and the run reaches the terminal output node.
#[test]
fn second_round_really_dispatches() -> TestResult {
    run_case(
        graph("hello", "fail", 10),
        json!(["a", "b"]),
        WorkflowRunStatus::Succeeded,
        2,
        None,
        None,
    )
}

/// Round-start bindings are committed before the first prompt is rendered.
#[test]
fn round_prompt_reads_item_and_index() -> TestResult {
    run_case(
        graph("item={{#iter.item#}} index={{#iter.index#}}", "fail", 10),
        json!(["a"]),
        WorkflowRunStatus::Succeeded,
        1,
        Some("item=a index=0"),
        None,
    )
}

/// A swift failure settles a fail-strategy iteration even when no async callback is pending.
#[test]
fn failed_condition_fails_the_iteration() -> TestResult {
    run_case(
        failing_condition_graph("fail"),
        json!(["a"]),
        WorkflowRunStatus::Failed,
        0,
        None,
        None,
    )
}

/// Continue absorbs that swift failure and completes the run without opening an Agent session.
#[test]
fn failed_condition_continue_completes_the_iteration() -> TestResult {
    run_case(
        failing_condition_graph("continue"),
        json!(["a"]),
        WorkflowRunStatus::Succeeded,
        0,
        None,
        None,
    )
}

/// A graph saved with editor geometry and the decorative entry handle publishes and executes all
/// three rounds through Condition → Agent, then exposes a three-item aggregate.
#[test]
fn editor_authored_multi_node_region_executes_three_rounds() -> TestResult {
    run_case(
        multi_node_region_graph(),
        json!(["a", "b", "c"]),
        WorkflowRunStatus::Succeeded,
        3,
        Some("item=c index=2"),
        Some(3),
    )
}

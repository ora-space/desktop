//! The swift control-node runtimes: Start, Condition, and Output.
//!
//! Each runtime here is the extraction of what was previously a `NodeType` match arm inside
//! `run_schedule`. They complete synchronously inside a scheduling wave against in-memory
//! committed facts and return the terminal decision for the engine to persist.

use super::{NodeRuntime, SwiftCompletion, SwiftNodeRuntime};
use crate::workflow_run::engine::condition::{ConditionError, evaluate_condition};
use crate::workflow_run::engine::graph::WorkflowGraphNode;
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::ExecutionContext;
use ora_domain::WorkflowNodeStatus;

/// The Start node runtime: records the run's kickoff input as the node's output.
pub(super) struct StartRuntime;

impl NodeRuntime for StartRuntime {
    /// The start node-run records the run's kickoff input as its scalar input.
    fn start_input(&self, _node: &WorkflowGraphNode, context: &ExecutionContext) -> Option<String> {
        context.run.input.clone()
    }

    /// The Start node records the kickoff input, not a terminal result.
    fn run_output_rank(&self) -> Option<u32> {
        None
    }
}

impl SwiftNodeRuntime for StartRuntime {
    fn complete_running(
        &self,
        _node: &WorkflowGraphNode,
        completion: &SwiftCompletion<'_>,
    ) -> Result<String, String> {
        Ok(completion.run_input.unwrap_or_default().to_string())
    }
}

/// The Condition node runtime: evaluates the committed cases and reports the selected branch.
pub(super) struct ConditionRuntime;

impl NodeRuntime for ConditionRuntime {
    fn start_input(
        &self,
        _node: &WorkflowGraphNode,
        _context: &ExecutionContext,
    ) -> Option<String> {
        None
    }

    /// A Condition routes the run; it never contributes the run output.
    fn run_output_rank(&self) -> Option<u32> {
        None
    }
}

impl SwiftNodeRuntime for ConditionRuntime {
    fn complete_running(
        &self,
        node: &WorkflowGraphNode,
        completion: &SwiftCompletion<'_>,
    ) -> Result<String, String> {
        // A condition that reads an unset or invalid variable fails the node and the run
        // rather than guessing a branch.
        node.condition_config
            .as_ref()
            .map(|config| evaluate_condition(config, completion.pool))
            .unwrap_or_else(|| Err(ConditionError::MissingConfig))
            .map_err(|error| error.to_string())
    }
}

/// The Output node runtime: resolves the run's declared result bindings into its output.
pub(super) struct OutputRuntime;

impl NodeRuntime for OutputRuntime {
    fn start_input(
        &self,
        _node: &WorkflowGraphNode,
        _context: &ExecutionContext,
    ) -> Option<String> {
        None
    }

    /// A completed Output node is the run's terminal result and outranks every other
    /// contributor (the Agent fallback declares rank 1).
    fn run_output_rank(&self) -> Option<u32> {
        Some(0)
    }
}

impl SwiftNodeRuntime for OutputRuntime {
    fn complete_running(
        &self,
        node: &WorkflowGraphNode,
        completion: &SwiftCompletion<'_>,
    ) -> Result<String, String> {
        // A workflow must reach exactly one Output; a second one completing means the active
        // path degenerated into two terminals.
        if let Some(previous) = completion.node_runs.iter().find(|candidate| {
            candidate.id != completion.node_run.id
                && candidate.node_type == NodeType::Output.as_str()
                && candidate.status == WorkflowNodeStatus::Succeeded
        }) {
            return Err(format!(
                "multiple active output nodes: {} and {}",
                previous.node_id, node.id
            ));
        }
        // An Output node resolves only its explicitly declared variable bindings into a JSON
        // object; without bindings it emits the empty string rather than importing predecessor
        // output implicitly.
        let Some(config) = &node.output_config else {
            return Ok(String::new());
        };
        let mut result = serde_json::Map::new();
        for binding in &config.outputs {
            let value = completion
                .pool
                .resolve(&binding.variable_selector)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    format!(
                        "result {} references unassigned variable {}",
                        binding.name,
                        binding.variable_selector.qualified()
                    )
                })?;
            result.insert(binding.name.clone(), value.clone());
        }
        Ok(serde_json::Value::Object(result).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_run::engine::condition::ELSE_BRANCH_ID;
    use crate::workflow_run::engine::graph::WorkflowGraph;
    use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
    use ora_domain::{AuditFields, WorkflowNodeRun, WorkflowNodeRunId, WorkflowRunId};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// A node-run row of the given type, currently running.
    fn running_node_run(node_id: &str, node_type: &str) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("node-run-{node_id}")),
            WorkflowRunId::new("run-1"),
            node_id,
            node_type,
            None,
            WorkflowNodeStatus::Running,
            None,
            None,
            None,
            None,
            Some(1),
            None,
            AuditFields::new(1, 1, false),
        )
    }

    /// A succeeded node-run row of the given type.
    fn succeeded_node_run(node_id: &str, node_type: &str) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("node-run-{node_id}")),
            WorkflowRunId::new("run-1"),
            node_id,
            node_type,
            None,
            WorkflowNodeStatus::Succeeded,
            None,
            None,
            None,
            None,
            Some(1),
            Some(2),
            AuditFields::new(1, 1, false),
        )
    }

    /// Builds the completion context for one running node over the given sibling rows.
    fn completion_for<'a>(
        node_run: &'a WorkflowNodeRun,
        run_input: Option<&'a str>,
        pool: &'a WorkflowVariablePool,
        node_runs: &'a [WorkflowNodeRun],
    ) -> SwiftCompletion<'a> {
        SwiftCompletion {
            node_run,
            run_input,
            pool,
            node_runs,
        }
    }

    /// An output node with declared bindings resolves each named result from the variable pool.
    #[test]
    fn output_resolves_declared_bindings_from_the_pool() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"review","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"review"}}},
                    {"id":"out","data":{"kind":"output","outputs":[
                        {"name":"approved","variableSelector":["review","structured_output","approved"]},
                        {"name":"files","variableSelector":["review","structured_output","files"]},
                        {"name":"summary","variableSelector":["review","text"]}
                    ]}}
                ],
                "edges": [{"source":"review","target":"out"}]
            }"#,
        )
        .unwrap();
        let mut pool = WorkflowVariablePool::default();
        pool.declare("review.structured_output", "object", "review");
        pool.declare("review.text", "string", "review");
        pool.set(
            "review.structured_output",
            "review",
            json!({
                "approved": true,
                "files": [{
                    "file_path": "src/vs/base/common/numbers.ts",
                    "lines": [{
                        "symbol": "formatTokenCount",
                        "start_line": 15,
                        "end_line": 26
                    }]
                }]
            }),
        )
        .unwrap();
        pool.set("review.text", "review", json!("ok")).unwrap();

        let node_run = running_node_run("out", "output");
        let node_runs = vec![node_run.clone()];
        let completion = completion_for(&node_run, Some("task"), &pool, &node_runs);
        let node = graph.node("out").unwrap();
        assert_eq!(
            OutputRuntime.complete_running(node, &completion).unwrap(),
            r#"{"approved":true,"files":[{"file_path":"src/vs/base/common/numbers.ts","lines":[{"symbol":"formatTokenCount","start_line":15,"end_line":26}]}],"summary":"ok"}"#
        );
    }

    /// An output binding that references an unassigned variable fails instead of emitting null.
    #[test]
    fn output_fails_when_a_binding_is_unassigned() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"out","data":{"kind":"output","outputs":[
                        {"name":"summary","variableSelector":["writer","text"]}
                    ]}}
                ],
                "edges": []
            }"#,
        )
        .unwrap();
        let mut pool = WorkflowVariablePool::default();
        // The variable is declared by the graph but the writer has not produced it yet.
        pool.declare("writer.text", "string", "writer");

        let node_run = running_node_run("out", "output");
        let node_runs = vec![node_run.clone()];
        let completion = completion_for(&node_run, None, &pool, &node_runs);
        let node = graph.node("out").unwrap();
        let error = OutputRuntime
            .complete_running(node, &completion)
            .unwrap_err();
        assert!(error.contains("unassigned variable writer.text"));
    }

    /// An output node without bindings does not import predecessor output implicitly.
    #[test]
    fn output_without_bindings_is_empty() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
                    {"id":"out","data":{"kind":"output"}}
                ],
                "edges": [{"source":"a","target":"out"}]
            }"#,
        )
        .unwrap();
        let node_run = running_node_run("out", "output");
        let node_runs = vec![node_run.clone()];
        let pool = WorkflowVariablePool::default();
        let completion = completion_for(&node_run, None, &pool, &node_runs);
        let node = graph.node("out").unwrap();
        assert_eq!(
            OutputRuntime.complete_running(node, &completion).unwrap(),
            ""
        );
    }

    /// A second output completing after a succeeded one fails the node instead of silently
    /// overwriting the run's terminal result.
    #[test]
    fn output_fails_when_another_output_already_succeeded() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"out-1","data":{"kind":"output"}},
                    {"id":"out-2","data":{"kind":"output"}}
                ],
                "edges": []
            }"#,
        )
        .unwrap();
        let first = succeeded_node_run("out-1", "output");
        let second = running_node_run("out-2", "output");
        let node_runs = vec![first, second.clone()];
        let pool = WorkflowVariablePool::default();
        let completion = completion_for(&second, None, &pool, &node_runs);
        let node = graph.node("out-2").unwrap();
        assert_eq!(
            OutputRuntime
                .complete_running(node, &completion)
                .unwrap_err(),
            "multiple active output nodes: out-1 and out-2"
        );
    }

    /// The Start runtime records the run's kickoff input, defaulting to the empty string.
    #[test]
    fn start_records_the_kickoff_input_as_its_output() {
        let graph = WorkflowGraph::parse(
            r#"{"nodes":[{"id":"start","data":{"kind":"start"}}],"edges":[]}"#,
        )
        .unwrap();
        let node = graph.node("start").unwrap();
        let node_run = running_node_run("start", "start");
        let node_runs = vec![node_run.clone()];
        let pool = WorkflowVariablePool::default();
        for (run_input, expected) in [(Some("kickoff"), "kickoff"), (None, "")] {
            let completion = completion_for(&node_run, run_input, &pool, &node_runs);
            assert_eq!(
                StartRuntime.complete_running(node, &completion).unwrap(),
                expected
            );
        }
    }

    /// A condition without executable cases reports the else branch; an authored case against
    /// the committed pool selects its branch id.
    #[test]
    fn condition_selects_the_branch_from_the_committed_pool() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"c","data":{"kind":"condition","cases":[
                        {"id":"approved","logic":"and","conditions":[
                            {"variableSelector":["review","text"],"operator":"not_empty","value":null}
                        ]}
                    ]}}
                ],
                "edges": []
            }"#,
        )
        .unwrap();
        let node = graph.node("c").unwrap();
        let node_run = running_node_run("c", "condition");
        let node_runs = vec![node_run.clone()];

        let mut pool = WorkflowVariablePool::default();
        pool.declare("review.text", "string", "review");
        let empty = completion_for(&node_run, None, &pool, &node_runs);
        assert_eq!(
            ConditionRuntime.complete_running(node, &empty).unwrap(),
            ELSE_BRANCH_ID
        );

        pool.set("review.text", "review", json!("done")).unwrap();
        let assigned = completion_for(&node_run, None, &pool, &node_runs);
        assert_eq!(
            ConditionRuntime.complete_running(node, &assigned).unwrap(),
            "approved"
        );
    }

    /// A condition node whose config is missing fails instead of guessing a branch.
    #[test]
    fn condition_without_config_fails() {
        // Parsing always attaches a config, so evaluate a hand-built node without one to pin
        // the fail-closed path the runtime must keep.
        let unconfigured = WorkflowGraphNode {
            id: "c".to_string(),
            node_type: NodeType::Condition,
            title: String::new(),
            description: String::new(),
            instruction: None,
            input_variables: Vec::new(),
            agent_config: None,
            condition_config: None,
            output_config: None,
            iteration_config: None,
        };
        let node_run = running_node_run("c", "condition");
        let node_runs = vec![node_run.clone()];
        let pool = WorkflowVariablePool::default();
        let completion = completion_for(&node_run, None, &pool, &node_runs);
        assert_eq!(
            ConditionRuntime
                .complete_running(&unconfigured, &completion)
                .unwrap_err(),
            ConditionError::MissingConfig.to_string()
        );
    }
}

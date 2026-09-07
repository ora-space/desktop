use crate::workflow_run::engine::graph::StartInputFieldType;
use crate::workflow_run::engine::{
    AgentConfig, AgentExecutor, AgentOutputContract, AgentSkill, GraphError, NodeType,
    StructuredTextExposure, UnknownNodeType, WorkflowGraph, WorkflowGraphNode,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::str::FromStr;

/// Parses a JSON value as a frozen workflow graph, failing the test on error.
fn parse(value: Value) -> Result<WorkflowGraph, GraphError> {
    WorkflowGraph::parse(&value.to_string())
}

/// Current Start controls retain their UI semantics while declaring matching pool types.
#[test]
fn parses_supported_start_field_types() {
    let graph = parse(json!({
        "nodes": [{
            "id": "start",
            "data": {
                "kind": "start",
                "inputVariables": [
                    { "name": "title", "fieldType": "text-input", "valueType": "string" },
                    { "name": "body", "fieldType": "paragraph", "valueType": "string" },
                    { "name": "kind", "fieldType": "select", "valueType": "string", "options": ["a", "b"] },
                    { "name": "score", "fieldType": "number", "valueType": "number" },
                    { "name": "approved", "fieldType": "checkbox", "valueType": "boolean" },
                    { "name": "source", "fieldType": "file", "valueType": "file" },
                    { "name": "sources", "fieldType": "file-list", "valueType": "array[file]" },
                    { "name": "metadata", "fieldType": "json", "valueType": "object" }
                ]
            }
        }],
        "edges": []
    }))
    .unwrap();
    assert_eq!(
        graph
            .start_node()
            .unwrap()
            .input_variables
            .iter()
            .map(|variable| variable.field_type)
            .collect::<Vec<_>>(),
        vec![
            StartInputFieldType::TextInput,
            StartInputFieldType::Paragraph,
            StartInputFieldType::Select,
            StartInputFieldType::Number,
            StartInputFieldType::Checkbox,
            StartInputFieldType::File,
            StartInputFieldType::FileList,
            StartInputFieldType::Json,
        ]
    );
}

/// A control cannot claim a pool type it does not produce.
#[test]
fn rejects_mismatched_start_field_value_type() {
    assert_eq!(
        parse(json!({
            "nodes": [{
                "id": "start",
                "data": {
                    "kind": "start",
                    "inputVariables": [{
                        "name": "approved",
                        "fieldType": "checkbox",
                        "valueType": "string"
                    }]
                }
            }],
            "edges": []
        }))
        .unwrap_err(),
        GraphError::InvalidStartVariables {
            node_id: "start".to_string(),
            reason: "variable approved field type does not produce declared type string"
                .to_string(),
        }
    );
}

/// A linear chain matching the demo shape: start → agent a → agent b → output-1.
fn linear_chain() -> Value {
    json!({
        "id": "wf-1",
        "name": "Demo",
        "description": "A linear demo",
        "updatedAt": "2026-08-07T00:00:00Z",
        "viewport": { "x": 0.0, "y": 0.0, "zoom": 1.0 },
        "nodes": [
            { "id": "start", "type": "workflow", "data": { "kind": "start", "title": "开始", "instruction": "input" } },
            { "id": "a", "type": "workflow", "data": { "kind": "agent", "agentConfig": {
                "schemaVersion": 3,
                "executor": { "agentCli": "open_code", "modelId": "model-1" },
                "roleId": "Researcher",
                "skills": [{ "skillId": "explore", "enabled": true }],
                "prompt": "do a"
            } } },
            { "id": "b", "type": "workflow", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "model-1" },
                "roleId": "Reviewer",
                "skills": [],
                "prompt": "do b"
            } } },
            { "id": "output-1", "type": "workflow", "data": { "kind": "output", "title": "输出", "instruction": "" } }
        ],
        "edges": [
            { "id": "e1", "source": "start", "target": "a" },
            { "id": "e2", "source": "a", "target": "b" },
            { "id": "e3", "source": "b", "target": "output-1" }
        ]
    })
}

/// Two parallel branches merging into one node: start → left/right → merge → out.
fn fan_in() -> Value {
    json!({
        "nodes": [
            { "id": "start", "data": { "kind": "start" } },
            { "id": "left", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" }, "roleId": "R", "skills": [], "prompt": "left"
            } } },
            { "id": "right", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" }, "roleId": "R", "skills": [], "prompt": "right"
            } } },
            { "id": "merge", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" }, "roleId": "R", "skills": [], "prompt": "merge"
            } } },
            { "id": "out", "data": { "kind": "output" } }
        ],
        "edges": [
            { "source": "start", "target": "left" },
            { "source": "start", "target": "right" },
            { "source": "left", "target": "merge" },
            { "source": "right", "target": "merge" },
            { "source": "merge", "target": "out" }
        ]
    })
}

/// Returns the ids of the given nodes in order.
fn ids(nodes: &[&WorkflowGraphNode]) -> Vec<String> {
    nodes.iter().map(|node| node.id.clone()).collect()
}

// ── Parse: valid shapes ──

#[test]
fn parses_valid_envelope_with_metadata() {
    let graph = parse(linear_chain()).unwrap();
    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 3);
    assert_eq!(graph.start_node().unwrap().id, "start");
    assert_eq!(ids(&graph.successors("start")), vec!["a"]);
    assert_eq!(ids(&graph.predecessors("output-1")), vec!["b"]);
}

/// The editor stores a Start node's initial prompt in `data.input`; parsing exposes it as the
/// node instruction so it can seed the run input and the reserved `{start}.input` selector.
#[test]
fn start_node_input_parses_from_data_input() {
    let graph = parse(json!({
        "nodes": [{
            "id": "start",
            "data": { "kind": "start", "input": "检查当前工作区的未提交改动" }
        }],
        "edges": []
    }))
    .unwrap();
    assert_eq!(
        graph.start_node().unwrap().instruction.as_deref(),
        Some("检查当前工作区的未提交改动")
    );
}

/// A legacy Start snapshot that predates `data.input` keeps parsing from `data.instruction`.
#[test]
fn start_node_instruction_falls_back_to_legacy_data_instruction() {
    let graph = parse(json!({
        "nodes": [{
            "id": "start",
            "data": { "kind": "start", "instruction": "legacy-prompt" }
        }],
        "edges": []
    }))
    .unwrap();
    assert_eq!(
        graph.start_node().unwrap().instruction.as_deref(),
        Some("legacy-prompt")
    );
}

#[test]
fn parses_agent_config_into_the_model() {
    let graph = parse(linear_chain()).unwrap();
    let expected = WorkflowGraphNode {
        id: "a".to_string(),
        node_type: NodeType::Agent,
        title: String::new(),
        description: String::new(),
        instruction: None,
        input_variables: Vec::new(),
        agent_config: Some(AgentConfig {
            executor: AgentExecutor {
                agent_cli: "open_code".to_string(),
                model_id: "model-1".to_string(),
            },
            role_id: Some("Researcher".to_string()),
            skills: vec![AgentSkill {
                skill_id: "explore".to_string(),
                enabled: true,
            }],
            prompt: "do a".to_string(),
            // The linear_chain fixture omits `interactive`, so the default must be false.
            interactive: false,
            // The linear_chain fixture omits `outputContract`, so the default must be `None`.
            output_contract: None,
        }),
        condition_config: None,
        output_config: None,
    };
    assert_eq!(*graph.node("a").unwrap(), expected);
}

#[test]
fn parses_interactive_agent_flag_with_a_default_of_false() {
    let graph = parse(json!({
        "nodes": [
            { "id": "start", "data": { "kind": "start" } },
            { "id": "a", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" },
                "roleId": "R", "skills": [], "prompt": "a", "interactive": true
            } } },
            { "id": "b", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" },
                "roleId": "R", "skills": [], "prompt": "b"
            } } },
            { "id": "out", "data": { "kind": "output" } }
        ],
        "edges": [
            { "source": "start", "target": "a" },
            { "source": "a", "target": "b" },
            { "source": "b", "target": "out" }
        ]
    }))
    .unwrap();
    assert_eq!(
        graph
            .node("a")
            .unwrap()
            .agent_config
            .as_ref()
            .unwrap()
            .interactive,
        true
    );
    assert_eq!(
        graph
            .node("b")
            .unwrap()
            .agent_config
            .as_ref()
            .unwrap()
            .interactive,
        false
    );
}

#[test]
fn parses_empty_graph_as_legal() {
    let graph = parse(json!({ "nodes": [], "edges": [] })).unwrap();
    assert_eq!(graph.node_count(), 0);
    assert_eq!(graph.edge_count(), 0);
    assert_eq!(graph.start_node(), None);
}

#[test]
fn rejects_user_global_without_an_initial_value() {
    assert_eq!(
        parse(json!({
            "nodes": [],
            "edges": [],
            "globalVariables": [
                { "name": "global.region", "valueType": "string" }
            ]
        }))
        .unwrap_err(),
        GraphError::InvalidGlobalVariables {
            reason: "global variable global.region must have an initial value".to_string()
        }
    );
}

/// File values use the canonical durable representation before entering a run.
#[test]
fn normalizes_file_values_and_rejects_unsafe_global_paths() {
    let graph = parse(json!({
        "nodes": [{
            "id": "start",
            "data": {
                "kind": "start",
                "inputVariables": [
                    { "name": "attachments", "valueType": "array[file]", "value": ["one.txt"] }
                ]
            }
        }],
        "edges": [],
        "globalVariables": [
            { "name": "global.template", "valueType": "file", "value": "docs/template.md" }
        ]
    }))
    .unwrap();
    assert_eq!(
        graph.start_node().unwrap().input_variables[0].value,
        Some(json!([
            { "kind": "workspace_file", "path": "one.txt" }
        ]))
    );
    assert_eq!(
        graph
            .global_variables()
            .iter()
            .find(|variable| variable.name == "global.template")
            .unwrap()
            .value,
        Some(json!({
            "kind": "workspace_file",
            "path": "docs/template.md"
        }))
    );

    assert!(matches!(
        parse(json!({
            "nodes": [],
            "edges": [],
            "globalVariables": [
                { "name": "global.template", "valueType": "file", "value": "../template.md" }
            ]
        })),
        Err(GraphError::InvalidGlobalVariables { .. })
    ));
}

// ── Parse: structural errors ──

#[test]
fn rejects_invalid_json() {
    assert_eq!(
        WorkflowGraph::parse("not json").unwrap_err(),
        GraphError::InvalidJson
    );
    assert_eq!(
        WorkflowGraph::parse("[]").unwrap_err(),
        GraphError::InvalidJson
    );
    assert_eq!(
        WorkflowGraph::parse("\"text\"").unwrap_err(),
        GraphError::InvalidJson
    );
}

#[test]
fn rejects_missing_nodes() {
    assert_eq!(
        parse(json!({ "edges": [] })).unwrap_err(),
        GraphError::MissingNodes
    );
}

#[test]
fn rejects_missing_edges() {
    assert_eq!(
        parse(json!({ "nodes": [] })).unwrap_err(),
        GraphError::MissingEdges
    );
}

#[test]
fn rejects_node_missing_id() {
    assert_eq!(
        parse(json!({ "nodes": [{ "data": { "kind": "start" } }], "edges": [] })).unwrap_err(),
        GraphError::InvalidNode {
            reason: "missing id".to_string()
        }
    );
}

#[test]
fn rejects_node_missing_node_type() {
    assert_eq!(
        parse(json!({ "nodes": [{ "id": "a", "data": { "title": "x" } }], "edges": [] }))
            .unwrap_err(),
        GraphError::InvalidNode {
            reason: "node a has no node type".to_string()
        }
    );
}

#[test]
fn rejects_unknown_node_type() {
    assert_eq!(
        parse(json!({ "nodes": [{ "id": "a", "data": { "kind": "bogus" } }], "edges": [] }))
            .unwrap_err(),
        GraphError::UnknownNodeType {
            node_id: "a".to_string(),
            value: "bogus".to_string()
        }
    );
}

#[test]
fn rejects_dangling_edge() {
    assert_eq!(
        parse(json!({
            "nodes": [{ "id": "start", "data": { "kind": "start" } }],
            "edges": [{ "source": "start", "target": "missing" }]
        }))
        .unwrap_err(),
        GraphError::DanglingEdge {
            node_id: "missing".to_string()
        }
    );
}

#[test]
fn rejects_cycle() {
    assert_eq!(
        parse(json!({
            "nodes": [
                { "id": "a", "data": { "kind": "start" } },
                { "id": "b", "data": { "kind": "agent", "agentConfig": {
                    "executor": { "agentCli": "c", "modelId": "m" }, "roleId": "R", "skills": [], "prompt": "b"
                } } }
            ],
            "edges": [
                { "source": "a", "target": "b" },
                { "source": "b", "target": "a" }
            ]
        }))
        .unwrap_err(),
        GraphError::CycleDetected
    );
}

#[test]
fn rejects_self_loop_as_cycle() {
    assert_eq!(
        parse(json!({
            "nodes": [{ "id": "a", "data": { "kind": "start" } }],
            "edges": [{ "source": "a", "target": "a" }]
        }))
        .unwrap_err(),
        GraphError::CycleDetected
    );
}

#[test]
fn rejects_multiple_start_nodes() {
    assert_eq!(
        parse(json!({
            "nodes": [
                { "id": "a", "data": { "kind": "start" } },
                { "id": "b", "data": { "kind": "start" } }
            ],
            "edges": []
        }))
        .unwrap_err(),
        GraphError::MultipleStartNodes
    );
}

#[test]
fn rejects_duplicate_node_id() {
    assert_eq!(
        parse(json!({
            "nodes": [
                { "id": "a", "data": { "kind": "start" } },
                { "id": "a", "data": { "kind": "output" } }
            ],
            "edges": []
        }))
        .unwrap_err(),
        GraphError::DuplicateNodeId {
            node_id: "a".to_string()
        }
    );
}

// ── Branch-aware parsing ──

/// Parses the `cases` wire array of a Condition node into its executable config.
#[test]
fn parses_condition_cases_into_the_model() {
    let graph = parse(json!({
        "nodes": [
            { "id": "start", "data": { "kind": "start" } },
            { "id": "c", "data": { "kind": "condition", "cases": [
                {
                    "id": "approved",
                    "logic": "and",
                    "conditions": [
                        { "variableSelector": ["review", "structured_output", "approved"], "operator": "is", "value": true }
                    ]
                },
                {
                    "id": "score-hi",
                    "logic": "or",
                    "conditions": [
                        { "variableSelector": ["review", "text"], "operator": "not_empty", "value": null },
                        { "variableSelector": ["review", "structured_output", "score"], "operator": "greater_than", "value": 90 }
                    ]
                }
            ] } }
        ],
        "edges": []
    }))
    .unwrap();
    let config = graph.node("c").unwrap().condition_config.as_ref().unwrap();
    assert_eq!(config.cases.len(), 2);
    assert_eq!(config.cases[0].id, "approved");
    assert_eq!(
        config.cases[0].logic,
        crate::workflow_run::engine::condition::ConditionLogic::And
    );
    assert_eq!(config.cases[0].conditions.len(), 1);
    assert_eq!(config.cases[1].id, "score-hi");
    assert_eq!(
        config.cases[1].logic,
        crate::workflow_run::engine::condition::ConditionLogic::Or
    );
    assert_eq!(
        config.cases[0].conditions[0].variable_selector.qualified(),
        "review.structured_output"
    );
}

/// A Condition node without a `cases` array compiles to an always-else config.
#[test]
fn parses_a_condition_without_cases_as_always_else() {
    let graph = parse(json!({
        "nodes": [{ "id": "c", "data": { "kind": "condition" } }],
        "edges": []
    }))
    .unwrap();
    assert_eq!(
        graph
            .node("c")
            .unwrap()
            .condition_config
            .as_ref()
            .unwrap()
            .cases
            .len(),
        0
    );
}

/// Rejects an unknown comparison operator inside a condition case.
#[test]
fn rejects_invalid_condition_operator() {
    assert_eq!(
        parse(json!({
            "nodes": [
                { "id": "c", "data": { "kind": "condition", "cases": [
                    { "id": "a", "logic": "and", "conditions": [
                        { "variableSelector": ["x", "y"], "operator": "bogus", "value": null }
                    ] }
                ] } }
            ],
            "edges": []
        }))
        .unwrap_err(),
        GraphError::InvalidCondition {
            node_id: "c".to_string(),
            reason: "condition rule has unknown operator bogus".to_string(),
        }
    );
}

/// Parses the source port handle on each edge, which selects the Condition branch it feeds.
#[test]
fn parses_edge_source_handles() {
    let graph = parse(json!({
        "nodes": [
            { "id": "c", "data": { "kind": "condition" } },
            { "id": "ok", "data": { "kind": "output" } },
            { "id": "no", "data": { "kind": "output" } }
        ],
        "edges": [
            { "source": "c", "sourceHandle": "approved", "target": "ok" },
            { "source": "c", "sourceHandle": "else", "target": "no" }
        ]
    }))
    .unwrap();
    let edges = graph.outgoing_edges("c");
    assert_eq!(edges.len(), 2);
    // Outgoing edges are sorted by target id, so the "no" else-branch edge comes first.
    assert_eq!(edges[0].source_handle.as_deref(), Some("else"));
    assert_eq!(edges[1].source_handle.as_deref(), Some("approved"));
    assert_eq!(edges[0].target, "no");
    assert_eq!(edges[1].target, "ok");
    assert_eq!(
        graph.incoming_edges("ok"),
        vec![crate::workflow_run::engine::graph::WorkflowGraphEdge {
            source: "c".to_string(),
            target: "ok".to_string(),
            source_handle: Some("approved".to_string()),
        }]
    );
}

// ── Structured output parsing ──

/// Parses the agent `outputContract` wire field into the domain contract.
#[test]
fn parses_structured_output_contract_into_the_model() {
    let schema = json!({ "type": "object", "properties": { "approved": { "type": "boolean" } }, "required": ["approved"] });
    let graph = parse(json!({
        "nodes": [
            { "id": "start", "data": { "kind": "start" } },
            { "id": "review", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" },
                "roleId": "R", "skills": [], "prompt": "review",
                "outputContract": {
                    "type": "structured",
                    "textExposure": "includeFinalText",
                    "schema": schema
                }
            } } }
        ],
        "edges": []
    }))
    .unwrap();
    let contract = graph
        .node("review")
        .unwrap()
        .agent_config
        .as_ref()
        .unwrap()
        .output_contract
        .as_ref()
        .unwrap();
    assert_eq!(
        contract,
        &AgentOutputContract::Structured {
            schema,
            text_exposure: StructuredTextExposure::IncludeFinalText,
        }
    );
}

/// Agent schemas are rejected at graph parsing instead of failing only after execution.
#[test]
fn rejects_invalid_structured_output_schema() {
    assert!(matches!(
        parse(json!({
            "nodes": [{ "id": "review", "data": {
                "kind": "agent",
                "agentConfig": {
                    "executor": { "agentCli": "open_code", "modelId": "m" },
                    "prompt": "review",
                    "outputContract": {
                        "type": "structured",
                        "schema": {
                            "type": "object",
                            "properties": { "created": { "type": "date" } }
                        }
                    }
                }
            } }],
            "edges": []
        })),
        Err(GraphError::InvalidNode { .. })
    ));
}

/// A missing structured-output setting leaves the node as raw-text-only.
#[test]
fn agent_without_output_contract_keeps_raw_text_only() {
    let graph = parse(json!({
        "nodes": [
            { "id": "a", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "open_code", "modelId": "m" },
                "roleId": "R", "skills": [], "prompt": "a"
            } } }
        ],
        "edges": []
    }))
    .unwrap();
    let config = graph.node("a").unwrap().agent_config.as_ref().unwrap();
    assert_eq!(config.output_contract, None);
}

// ── Output binding parsing ──

/// Parses the output node's `data.outputs` bindings into named variable selectors.
#[test]
fn parses_output_bindings_into_the_model() {
    let graph = parse(json!({
        "nodes": [
            { "id": "out", "data": { "kind": "output", "outputs": [
                { "name": "approved", "variableSelector": ["review", "structured_output", "approved"] },
                { "name": "summary", "variableSelector": ["writer", "text"] }
            ] } }
        ],
        "edges": []
    }))
    .unwrap();
    let config = graph.node("out").unwrap().output_config.as_ref().unwrap();
    assert_eq!(config.outputs.len(), 2);
    assert_eq!(config.outputs[0].name, "approved");
    assert_eq!(
        config.outputs[0].variable_selector.qualified(),
        "review.structured_output"
    );
    assert_eq!(config.outputs[1].name, "summary");
    assert_eq!(
        config.outputs[1].variable_selector.qualified(),
        "writer.text"
    );
}

/// An output node without `outputs` keeps the legacy concatenated output.
#[test]
fn output_without_bindings_has_no_output_config() {
    let graph = parse(json!({
        "nodes": [{ "id": "out", "data": { "kind": "output" } }],
        "edges": []
    }))
    .unwrap();
    assert_eq!(graph.node("out").unwrap().output_config, None);
}

// ── Topology ──

#[test]
fn transitive_successors_follow_flow_order() {
    let graph = parse(linear_chain()).unwrap();
    assert_eq!(
        ids(&graph.transitive_successors("start")),
        vec!["a", "b", "output-1"]
    );
}

#[test]
fn transitive_predecessors_follow_upstream_first_order() {
    let graph = parse(linear_chain()).unwrap();
    assert_eq!(
        ids(&graph.transitive_predecessors("output-1")),
        vec!["start", "a", "b"]
    );
}

#[test]
fn all_nodes_follow_topological_execution_order() {
    let graph = parse(linear_chain()).unwrap();
    assert_eq!(
        ids(&graph.nodes_in_topological_order()),
        vec!["start", "a", "b", "output-1"]
    );
}

#[test]
fn transitive_successors_exclude_the_seed() {
    let graph = parse(fan_in()).unwrap();
    let successors = ids(&graph.transitive_successors("start"));
    let mut sorted = successors.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["left", "merge", "out", "right"]);
    assert!(!successors.contains(&"start".to_string()));
}

#[test]
fn transitive_predecessors_of_fan_in_are_stable_and_upstream_first() {
    let graph = parse(fan_in()).unwrap();
    let predecessors = ids(&graph.transitive_predecessors("merge"));
    let mut sorted = predecessors.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["left", "right", "start"]);
    // The start node has the lowest topological rank and must lead the lineage.
    assert_eq!(predecessors[0], "start");
    // Two queries return the identical order, pinning determinism.
    assert_eq!(predecessors, ids(&graph.transitive_predecessors("merge")));
}

#[test]
fn ready_set_starts_with_the_start_node() {
    let graph = parse(linear_chain()).unwrap();
    let completed = HashSet::new();
    assert_eq!(ids(&graph.ready_set(&completed)), vec!["start"]);
}

#[test]
fn ready_set_advances_along_the_chain() {
    let graph = parse(linear_chain()).unwrap();
    let completed = HashSet::from(["start"]);
    assert_eq!(ids(&graph.ready_set(&completed)), vec!["a"]);
}

#[test]
fn ready_set_handles_fan_in() {
    let graph = parse(fan_in()).unwrap();
    let completed = HashSet::from(["start", "left", "right"]);
    assert_eq!(ids(&graph.ready_set(&completed)), vec!["merge"]);
}

#[test]
fn fan_in_is_not_ready_until_every_predecessor_completes() {
    let graph = parse(fan_in()).unwrap();
    let completed = HashSet::from(["start", "left"]);
    // `right` becomes ready, but `merge` must wait for `right`.
    assert_eq!(ids(&graph.ready_set(&completed)), vec!["right"]);
}

#[test]
fn first_unsupported_node_reports_tool() {
    let graph = parse(json!({
        "nodes": [
            { "id": "start", "data": { "kind": "start" } },
            { "id": "t", "data": { "kind": "tool", "tool": "Terminal" } }
        ],
        "edges": []
    }))
    .unwrap();
    assert_eq!(graph.first_unsupported_node().unwrap().id, "t");
}

#[test]
fn first_unsupported_node_is_none_for_supported_graphs() {
    let graph = parse(linear_chain()).unwrap();
    assert_eq!(graph.first_unsupported_node(), None);
}

#[test]
fn unreachable_from_start_reports_isolated_nodes() {
    let graph = parse(json!({
        "nodes": [
            { "id": "start", "data": { "kind": "start" } },
            { "id": "a", "data": { "kind": "output" } },
            { "id": "orphan", "data": { "kind": "agent", "agentConfig": {
                "executor": { "agentCli": "c", "modelId": "m" }, "roleId": "R", "skills": [], "prompt": "orphan"
            } } }
        ],
        "edges": [{ "source": "start", "target": "a" }]
    }))
    .unwrap();
    assert_eq!(graph.unreachable_from_start(), vec!["orphan"]);
}

#[test]
fn unreachable_from_start_is_empty_for_a_connected_graph() {
    let graph = parse(linear_chain()).unwrap();
    assert_eq!(graph.unreachable_from_start(), Vec::<String>::new());
}

// ── Node type registry ──

#[test]
fn node_type_round_trips_all_variants() {
    for (value, expected) in [
        ("start", NodeType::Start),
        ("agent", NodeType::Agent),
        ("prompt", NodeType::Prompt),
        ("condition", NodeType::Condition),
        ("tool", NodeType::Tool),
        ("output", NodeType::Output),
    ] {
        let parsed = NodeType::from_str(value).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_str(), value);
    }
}

#[test]
fn node_type_rejects_unknown_values() {
    assert_eq!(
        NodeType::from_str("bogus"),
        Err(UnknownNodeType("bogus".to_string()))
    );
}

#[test]
fn node_type_reports_the_v1_supported_set() {
    let supported: Vec<&str> = [
        NodeType::Start,
        NodeType::Agent,
        NodeType::Prompt,
        NodeType::Condition,
        NodeType::Tool,
        NodeType::Output,
    ]
    .iter()
    .filter(|node_type| node_type.supported())
    .map(|node_type| node_type.as_str())
    .collect();
    assert_eq!(supported, vec!["start", "agent", "condition", "output"]);
}

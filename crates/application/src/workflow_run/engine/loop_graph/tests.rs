use super::*;
use pretty_assertions::assert_eq;

/// Exercises the public snapshot decoder with separate outer and round entry nodes.
fn snapshot() -> Value {
    json!({
        "schemaVersion": 2,
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "loop", "data": {"kind": "loop", "loopConfig": {
                "maxIterations": 5,
                "variables": [{"name": "draft", "valueType": "string",
                    "initial": {"kind": "constant", "value": ""},
                    "feedback": ["writer", "output"]}],
                "until": {"logic": "and", "conditions": [{
                    "variableSelector": ["writer", "output"],
                    "operator": "equals", "value": "done"
                }]},
                "outputs": [{"name": "result", "variableSelector": ["writer", "output"]}]
            }}},
            {"id": "entry", "parentId": "loop", "data": {"kind": "start", "containerId": "loop"}},
            {"id": "writer", "data": {"kind": "agent", "containerId": "loop", "agentConfig": {
                "executor": {"agentCli": "claude", "modelId": "model"}, "prompt": "Write"
            }}}
        ],
        "edges": [{"source": "start", "target": "loop"}, {"source": "entry", "target": "writer"}]
    })
}

/// Scope queries cannot expose child nodes as independently schedulable outer nodes.
#[test]
fn partitions_round_topology() {
    let graph = WorkflowGraph::parse(&snapshot().to_string()).unwrap();
    let (config, body) = graph.loop_body("loop").unwrap();
    assert_eq!(
        (
            graph.node_count(),
            graph.edge_count(),
            body.node_count(),
            body.edge_count()
        ),
        (2, 1, 2, 1)
    );
    assert_eq!(graph.node("writer"), None);
    assert_eq!(
        graph.execution_node("writer").map(|node| node.id.as_str()),
        Some("writer")
    );
    assert_eq!(
        body.start_node().map(|node| node.id.as_str()),
        Some("entry")
    );
    assert_eq!(config.max_iterations, 5);
    assert_eq!(
        config.variables[0].initial,
        super::super::loop_config::LoopInitialValue::Constant(json!(""))
    );
}

/// An unfinished spare child does not become an independent round entry or affect bindings.
#[test]
fn excludes_unused_loop_members() {
    let mut source = snapshot();
    source["nodes"].as_array_mut().unwrap().push(json!({
        "id":"spare", "data":{"kind":"agent", "containerId":"loop", "agentConfig":false}
    }));
    source["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":"spare","target":"writer"}));
    let source = source.to_string();
    assert_eq!(
        WorkflowGraph::unused_node_ids(&source).unwrap(),
        vec!["spare"]
    );
    let graph = WorkflowGraph::parse(&source).unwrap();
    assert_eq!(graph.loop_body("loop").unwrap().1.node_count(), 2);
    assert_eq!(graph.execution_node("spare"), None);
}

/// Flat snapshots retain their original decoding behavior without a version marker.
#[test]
fn preserves_flat_snapshots() {
    let graph =
        WorkflowGraph::parse(r#"{"nodes":[{"id":"start","data":{"kind":"start"}}],"edges":[]}"#)
            .unwrap();
    assert_eq!((graph.node_count(), graph.edge_count()), (1, 0));
    assert!(graph.loop_body("loop").is_none());
}

/// Outer consumers use exports, and initializers read only upstream outer nodes.
#[test]
fn enforces_binding_visibility() {
    let mut value = snapshot();
    value["nodes"][1]["data"]["loopConfig"]["variables"][0]["initial"] =
        json!({"kind": "variable", "selector": ["start", "input"]});
    value["nodes"].as_array_mut().unwrap().push(json!({
        "id": "output", "data": {"kind": "output", "outputs": [
            {"name": "result", "variableSelector": ["loop", "result"]}
        ]}
    }));
    value["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source": "loop", "target": "output"}));
    assert!(WorkflowGraph::parse(&value.to_string()).is_ok());
    value["nodes"][4]["data"]["outputs"][0]["variableSelector"] = json!(["writer", "output"]);
    assert!(WorkflowGraph::parse(&value.to_string()).is_err());
    for source in ["writer", "output", "loop", "missing"] {
        let mut value = snapshot();
        value["nodes"][1]["data"]["loopConfig"]["variables"][0]["initial"] =
            json!({"kind": "variable", "selector": [source, "output"]});
        assert!(
            WorkflowGraph::parse(&value.to_string()).is_err(),
            "accepted initializer from {source}"
        );
    }
}

/// Both graph levels must remain DAGs; feedback is represented only by explicit bindings.
#[test]
fn rejects_cycles_in_each_scope() {
    for edge in [
        json!({"source": "loop", "target": "start"}),
        json!({"source": "writer", "target": "entry"}),
    ] {
        let mut value = snapshot();
        value["edges"].as_array_mut().unwrap().push(edge);
        assert!(WorkflowGraph::parse(&value.to_string()).is_err());
    }
}

/// Ownership errors are rejected before a child could escape its container lifetime.
#[test]
fn rejects_invalid_ownership_and_cross_scope_edges() {
    for (pointer, replacement) in [
        ("/schemaVersion", json!(1)),
        ("/nodes/2/parentId", json!("other")),
        ("/nodes/2/data/containerId", json!("missing")),
        ("/nodes/2/data/containerId", Value::Null),
        ("/nodes/1/data/containerId", json!("loop")),
        ("/nodes/3/id", json!("start")),
        ("/edges/0/target", json!("writer")),
        ("/edges/1/target", json!("missing")),
        ("/nodes/2/data/kind", json!("agent")),
        ("/nodes/3/data/kind", json!("output")),
    ] {
        let mut value = snapshot();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[field] = replacement;
        assert!(
            WorkflowGraph::parse(&value.to_string()).is_err(),
            "accepted {pointer}: {value}"
        );
    }
    let mut value = snapshot();
    value["edges"].as_array_mut().unwrap().pop();
    assert!(WorkflowGraph::parse(&value.to_string()).is_err());
}

/// Invalid limits, values, conditions and bindings cannot enter a frozen executable snapshot.
#[test]
fn rejects_invalid_loop_configuration() {
    for (pointer, replacement) in [
        ("/maxIterations", json!(0)),
        ("/maxIterations", json!(101)),
        ("/maxIterations", json!(1.5)),
        ("/variables/0/name", json!("bad.name")),
        ("/variables/0/valueType", json!("unknown")),
        ("/variables/0/initial/value", json!(false)),
        ("/variables/0/feedback", json!(["writer"])),
        ("/until/logic", json!("xor")),
        ("/until/conditions/0/operator", json!("unknown")),
        ("/outputs/0/name", json!(" ")),
        ("/outputs/0/variableSelector", json!([])),
    ] {
        let mut value = snapshot();
        *value["nodes"][1]["data"]["loopConfig"]
            .pointer_mut(pointer)
            .unwrap() = replacement;
        assert!(
            WorkflowGraph::parse(&value.to_string()).is_err(),
            "accepted {pointer}: {value}"
        );
    }
    for field in ["variables", "outputs"] {
        let mut value = snapshot();
        let entries = value["nodes"][1]["data"]["loopConfig"][field]
            .as_array_mut()
            .unwrap();
        entries.push(entries[0].clone());
        assert!(WorkflowGraph::parse(&value.to_string()).is_err());
    }
}

/// Required configuration is explicit, so typos cannot silently change execution behavior.
#[test]
fn rejects_missing_and_unknown_config_fields() {
    for field in ["maxIterations", "variables", "until", "outputs"] {
        let mut value = snapshot();
        value["nodes"][1]["data"]["loopConfig"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            WorkflowGraph::parse(&value.to_string()).is_err(),
            "accepted missing {field}"
        );
    }
    let mut value = snapshot();
    value["nodes"][1]["data"]["loopConfig"]["maxIteration"] = json!(5);
    assert!(WorkflowGraph::parse(&value.to_string()).is_err());
}

/// Both bounds are inclusive and remain executable after graph partitioning.
#[test]
fn accepts_executable_iteration_bounds() {
    for limit in [1, 100] {
        let mut value = snapshot();
        value["nodes"][1]["data"]["loopConfig"]["maxIterations"] = json!(limit);
        let graph = WorkflowGraph::parse(&value.to_string()).unwrap();
        assert_eq!(graph.loop_body("loop").unwrap().0.max_iterations, limit);
        assert_eq!(graph.first_unsupported_node(), None);
    }
}

/// Generated qualification scenarios must satisfy the real decoder before any Agent is started.
#[test]
fn qualification_generator_obeys_execution_graph_contract() {
    ora_logging::with_trace_logging(|| {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("application crate belongs to the repository");
        let pinned_version = std::fs::read_to_string(repository.join(".deno-version"))
            .expect("repository pins the fixture runtime");
        let runtime_version = std::process::Command::new("deno")
            .arg("--version")
            .output()
            .expect("pinned Deno is available for generator contract tests");
        assert!(runtime_version.status.success());
        assert_eq!(
            String::from_utf8_lossy(&runtime_version.stdout)
                .split_whitespace()
                .nth(1),
            Some(pinned_version.trim())
        );
        let fixture = repository
            .join("scripts")
            .join("workflow-qualification-suite")
            .join("export-graphs.mjs")
            .canonicalize()
            .expect("generator fixture export exists");
        let generated = std::process::Command::new("deno")
            .args(["run", "--no-config", "--no-lock"])
            .arg(fixture)
            .output()
            .expect("generate qualification graphs without opening a database");
        assert!(
            generated.status.success(),
            "generator failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let scenarios: Vec<Value> = serde_json::from_slice(&generated.stdout)
            .expect("fixture export returns the complete generated scenario array");
        assert_eq!(
            scenarios
                .iter()
                .map(|scenario| scenario["index"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            (1..=101).collect::<Vec<_>>()
        );
        for scenario in scenarios {
            let index = &scenario["index"];
            let source = scenario["graph"].to_string();
            let graph = WorkflowGraph::parse(&source).unwrap_or_else(|error| {
                panic!("generated scenario {index} fails the decoder: {error}")
            });
            assert_eq!(
                (
                    graph
                        .execution_scopes()
                        .into_iter()
                        .map(WorkflowGraph::node_count)
                        .sum::<usize>(),
                    WorkflowGraph::unused_node_ids(&source).unwrap(),
                    graph.first_unsupported_node().is_none(),
                ),
                (
                    scenario["graph"]["nodes"].as_array().unwrap().len(),
                    Vec::<String>::new(),
                    true,
                ),
                "generated scenario {index} must retain its complete executable graph"
            );
        }
    });
}

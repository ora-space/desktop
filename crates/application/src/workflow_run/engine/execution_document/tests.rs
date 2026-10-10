use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

/// Region membership must be derived after pruning, including edges from spare members.
#[test]
fn excludes_unused_iteration_members_and_revalidates_reconnected_nodes() {
    let mut source = json!({"nodes":[
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{
            "iteratorSelector":["start","items"],"collectSelector":["fix","output"]
        }}},
        {"id":"fix","parentId":"iter","data":{"kind":"agent"}},
        {"id":"spare","parentId":"iter","data":{"kind":"agent","agentConfig":false}},
        {"id":"out","data":{"kind":"output"}}
    ],"edges":[
        {"source":"start","target":"iter"},
        {"source":"iter","target":"fix"},
        {"source":"iter","target":"out"},
        {"source":"spare","target":"fix"}
    ]});
    let graph = WorkflowGraph::parse(&source.to_string()).unwrap();
    assert_eq!(graph.region("iter").unwrap().member_ids, vec!["fix"]);
    assert_eq!(
        WorkflowGraph::unused_node_ids(&source.to_string()).unwrap(),
        vec!["spare"]
    );
    source["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":"iter","target":"spare"}));
    assert!(WorkflowGraph::parse(&source.to_string()).is_err());
    assert_eq!(
        WorkflowGraph::unused_node_ids(&source.to_string()).unwrap(),
        Vec::<String>::new()
    );
}

/// Spare configurations and cycles must not enter the executable graph or block a join.
#[test]
fn excludes_spare_chains_and_their_incoming_join_edges() {
    let source = json!({"nodes": [
        {"id":"start","data":{"kind":"start"}},
        {"id":"out","data":{"kind":"output"}},
        {"id":"spare","data":{"kind":"agent","agentConfig":"unfinished"}},
        {"id":"other","data":{"kind":"condition","cases":"unfinished"}}
    ], "edges": [
        {"source":"start","target":"out"},
        {"source":"spare","target":"other"},
        {"source":"other","target":"spare"},
        {"source":"spare","target":"out"}
    ]})
    .to_string();
    assert_eq!(
        WorkflowGraph::unused_node_ids(&source).unwrap(),
        vec!["other", "spare"]
    );
    let graph = WorkflowGraph::parse(&source).unwrap();
    assert_eq!(
        graph
            .nodes()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["start", "out"]
    );
    assert_eq!(
        graph
            .incoming_edges("out")
            .into_iter()
            .map(|edge| edge.source)
            .collect::<Vec<_>>(),
        vec!["start"]
    );
}

/// Membership analysis does not parse unused Loop configuration or its body.
#[test]
fn excludes_an_entire_unused_container() {
    let source = json!({"schemaVersion":2,"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"loop","data":{"kind":"loop"}},
        {"id":"entry","data":{"kind":"start","containerId":"loop"}},
        {"id":"agent","data":{"kind":"agent","containerId":"loop","agentConfig":false}}
    ],"edges":[{"source":"entry","target":"agent"}]})
    .to_string();
    assert_eq!(
        WorkflowGraph::unused_node_ids(&source).unwrap(),
        vec!["agent", "entry", "loop"]
    );
    assert_eq!(WorkflowGraph::parse(&source).unwrap().nodes().count(), 1);
}

/// References to spare output fail before any consumer prepares runtime dependencies.
#[test]
fn rejects_active_prompt_referencing_unused_output() {
    let source = json!({"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"agent","data":{"kind":"agent","agentConfig":{"prompt":"{{#spare.text#}}"}}},
        {"id":"spare","data":{"kind":"agent"}}
    ],"edges":[{"source":"start","target":"agent"}]})
    .to_string();
    assert_eq!(
        WorkflowGraph::parse(&source).unwrap_err(),
        GraphError::InvalidNode {
            reason: "variable references unused node spare".into(),
        }
    );
}

/// Filtering cannot hide dangling edges even when their source is unused.
#[test]
fn rejects_invalid_document_structure_before_filtering() {
    let source = json!({"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"spare","data":{"kind":"agent"}}
    ],"edges":[{"source":"spare","target":"missing"}]})
    .to_string();
    assert_eq!(
        WorkflowGraph::parse(&source).unwrap_err(),
        GraphError::DanglingEdge {
            node_id: "missing".into()
        }
    );
}

/// Dormant containers still own their boundaries; filtering must not conceal invalid ownership.
#[test]
fn rejects_invalid_ownership_and_cross_scope_edges_on_spare_nodes() {
    let mut source = json!({"schemaVersion":2,"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"loop","data":{"kind":"loop"}},
        {"id":"spare","data":{"kind":"agent","containerId":"loop"}}
    ],"edges":[{"source":"spare","target":"start"}]});
    assert_eq!(
        WorkflowGraph::parse(&source.to_string()).unwrap_err(),
        GraphError::InvalidRegion {
            node_id: "spare".into(),
            reason: "edge crosses a container boundary".into(),
        }
    );
    source["edges"] = json!([]);
    source["nodes"][2]["data"]["containerId"] = Value::Null;
    assert_eq!(
        WorkflowGraph::parse(&source.to_string()).unwrap_err(),
        GraphError::InvalidNode {
            reason: "containerId must be a non-empty string".into(),
        }
    );
}

/// An unrecognized kind is not reported as a spare node the author chose to keep.
#[test]
fn unrecognized_kinds_are_separate_from_spare_nodes() {
    let source = json!({"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"spare","data":{"kind":"agent"}},
        {"id":"weird","data":{"kind":"hologram"}}
    ],"edges":[]});
    assert_eq!(
        WorkflowGraph::authoring_participation(&source.to_string()).unwrap(),
        AuthoringParticipation {
            unused_node_ids: vec!["spare".into()],
            unrecognized_nodes: vec![UnrecognizedAuthoringNode {
                node_id: "weird".into(),
                kind: "hologram".into(),
            }],
        }
    );
    // Execution still treats the dropped kind as unused so a reference to it fails closed.
    assert_eq!(
        WorkflowGraph::unused_node_ids(&source.to_string()).unwrap(),
        vec!["spare".to_string(), "weird".to_string()]
    );
}

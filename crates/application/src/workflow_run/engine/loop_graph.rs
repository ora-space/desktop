//! Partitions container graphs before the existing DAG parser applies scope-local invariants.

use super::graph::{GraphError, WorkflowGraph};
use super::loop_config::LoopConfig;
use super::node_type::NodeType;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

impl WorkflowGraph {
    /// Parses a frozen React Flow graph JSON into a validated, scope-aware DAG.
    pub fn parse(source: &str) -> Result<Self, GraphError> {
        let document = super::execution_document::project(source)?;
        let graph = parse_scoped_graph(&document.graph.to_string())?;
        super::unused_references::validate(&graph, &document.unused_node_ids)?;
        Ok(graph)
    }

    /// Finds a node in the root graph or any container body.
    pub fn execution_node(&self, id: &str) -> Option<&super::graph::WorkflowGraphNode> {
        self.node(id).or_else(|| {
            self.nodes()
                .filter_map(|node| self.loops.get(&node.id).map(|(_, body)| body))
                .find_map(|body| body.execution_node(id))
        })
    }

    /// Returns a container's frozen configuration and its single-round DAG.
    pub fn loop_body(&self, id: &str) -> Option<(&LoopConfig, &Self)> {
        self.loops.get(id).map(|(config, graph)| (config, graph))
    }

    /// Returns the id of the Loop whose body declares `node_id`, if any.
    pub fn loop_owner(&self, node_id: &str) -> Option<&str> {
        self.loops
            .iter()
            .find(|(_, (_, body))| body.node(node_id).is_some())
            .map(|(owner, _)| owner.as_str())
    }

    /// Returns this graph followed by each container body in deterministic outer-node order.
    pub fn execution_scopes(&self) -> Vec<&Self> {
        let mut scopes = vec![self];
        for node in self.nodes() {
            if let Some((_, body)) = self.loops.get(&node.id) {
                scopes.extend(body.execution_scopes());
            }
        }
        scopes
    }

    /// Returns the first unsupported root or container node in deterministic graph order.
    pub fn first_unsupported_node(&self) -> Option<&super::graph::WorkflowGraphNode> {
        if let Some(node) = self.nodes().find(|node| !node.node_type.supported()) {
            return Some(node);
        }
        self.nodes()
            .filter_map(|node| self.loops.get(&node.id).map(|(_, body)| body))
            .find_map(Self::first_unsupported_node)
    }
}

/// Decodes flat snapshots unchanged and validates explicit ownership for version-two graphs.
pub(super) fn parse_scoped_graph(source: &str) -> Result<WorkflowGraph, GraphError> {
    let envelope: Value = serde_json::from_str(source).map_err(|_| GraphError::InvalidJson)?;
    let has_containers = envelope["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|node| node["data"]["kind"] == "loop" || node["data"].get("containerId").is_some());
    if !has_containers {
        return WorkflowGraph::parse_flat(source);
    }
    let nodes = envelope["nodes"]
        .as_array()
        .ok_or(GraphError::MissingNodes)?;
    let edges = envelope["edges"]
        .as_array()
        .ok_or(GraphError::MissingEdges)?;
    if envelope["schemaVersion"] != 2 {
        return Err(invalid("container graphs require schemaVersion 2"));
    }

    let mut owners = HashMap::new();
    let mut configs = BTreeMap::new();
    let mut scoped_nodes: BTreeMap<Option<String>, Vec<Value>> = BTreeMap::new();
    for node in nodes {
        let id = node["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| invalid("node has no non-empty ID"))?;
        let owner = match node["data"].get("containerId") {
            None => None,
            Some(Value::String(owner)) if !owner.trim().is_empty() => Some(owner.clone()),
            Some(_) => {
                return Err(invalid(
                    "containerId must be a non-empty string when present",
                ));
            }
        };
        if owners.insert(id.to_string(), owner.clone()).is_some() {
            return Err(GraphError::DuplicateNodeId { node_id: id.into() });
        }
        if node["data"]["kind"] == "loop" {
            if owner.is_some() || node.get("parentId").is_some() {
                return Err(invalid("nested Loops are not supported"));
            }
            let config = LoopConfig::parse(node["data"]["loopConfig"].clone())
                .map_err(|reason| invalid(&format!("loop {id}: {reason}")))?;
            configs.insert(id.to_string(), config);
        }
        if owner.is_some() && node["data"]["kind"] == "iteration" {
            return Err(invalid("nested composite nodes are not supported"));
        }
        // Renderer parentage is derived by the editor and must agree when serialized.
        if owner.is_some()
            && let Some(parent) = node.get("parentId")
            && parent.as_str() != owner.as_deref()
        {
            return Err(invalid("parentId disagrees with data.containerId"));
        }
        let mut scoped_node = node.clone();
        if owner.is_some()
            && let Some(object) = scoped_node.as_object_mut()
        {
            object.remove("parentId");
        }
        scoped_nodes.entry(owner).or_default().push(scoped_node);
    }
    for owner in owners.values().flatten() {
        if !configs.contains_key(owner) {
            return Err(invalid(&format!("unknown Loop owner {owner}")));
        }
    }
    let mut scoped_edges: BTreeMap<Option<String>, Vec<Value>> = BTreeMap::new();
    for edge in edges {
        let source = edge["source"]
            .as_str()
            .ok_or_else(|| invalid("edge has no source"))?;
        let target = edge["target"]
            .as_str()
            .ok_or_else(|| invalid("edge has no target"))?;
        let source_owner = owners.get(source).ok_or_else(|| GraphError::DanglingEdge {
            node_id: source.into(),
        })?;
        let target_owner = owners.get(target).ok_or_else(|| GraphError::DanglingEdge {
            node_id: target.into(),
        })?;
        if source_owner != target_owner {
            return Err(invalid("edges cannot cross Loop boundaries"));
        }
        scoped_edges
            .entry(source_owner.clone())
            .or_default()
            .push(edge.clone());
    }
    let globals = envelope
        .get("globalVariables")
        .or_else(|| envelope.get("global_variables"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let mut root = WorkflowGraph::parse_flat(
        &json!({
            "nodes": scoped_nodes.remove(&None).unwrap_or_default(),
            "edges": scoped_edges.remove(&None).unwrap_or_default(),
            "globalVariables": globals,
        })
        .to_string(),
    )?;
    validate_scope(&root)?;
    for (id, config) in configs {
        let owner = Some(id.clone());
        let body = WorkflowGraph::parse_flat(
            &json!({
                "nodes": scoped_nodes.remove(&owner).unwrap_or_default(),
                "edges": scoped_edges.remove(&owner).unwrap_or_default(),
                "globalVariables": globals,
            })
            .to_string(),
        )?;
        validate_scope(&body)?;
        if body.nodes().any(|node| node.node_type == NodeType::Output) {
            return Err(invalid(
                "Loop bodies export results through loopConfig.outputs",
            ));
        }
        root.loops.insert(id, (config, body));
    }
    super::loop_bindings::validate(&root)?;
    Ok(root)
}

/// A container entry is explicit so graph ordering never guesses which independent root to run.
fn validate_scope(graph: &WorkflowGraph) -> Result<(), GraphError> {
    if graph.start_node().is_none() {
        return Err(invalid("every container scope requires one Start node"));
    }
    let unreachable = graph.unreachable_from_start();
    if !unreachable.is_empty() {
        return Err(invalid(&format!(
            "unreachable scope nodes: {unreachable:?}"
        )));
    }
    Ok(())
}

/// Keeps parse failures on the existing typed graph boundary used by deployment errors.
fn invalid(reason: &str) -> GraphError {
    GraphError::InvalidNode {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests;

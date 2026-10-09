//! Separates the authoring document from the entry-reachable execution document.
//!
//! Only structural fields are decoded before membership is known: an unfinished spare
//! Agent or container must not require executable configuration or installed dependencies.

use super::graph::{GraphError, WorkflowGraph};
use super::node_type::NodeType;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

struct Node<'a> {
    id: &'a str,
    kind: &'a str,
    owner: Option<&'a str>,
}

/// A node whose `data.kind` is not a kind this version can execute or keep as a spare.
///
/// Spare nodes are reachable-set leftovers with a known kind. An unrecognized kind is a
/// different fact: the canvas drops it on load, and analyze must not describe that drop as
/// an author choosing to leave a node off the execution path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecognizedAuthoringNode {
    pub node_id: String,
    pub kind: String,
}

/// Participation split for editor and run-overview analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringParticipation {
    /// Known kinds that are outside the entry-reachable execution subgraph.
    pub unused_node_ids: Vec<String>,
    /// Nodes whose kind string is not registered, whether or not an edge reaches them.
    pub unrecognized_nodes: Vec<UnrecognizedAuthoringNode>,
}

/// A deterministic projection; the original JSON is never rewritten in persistence.
pub(super) struct ExecutionDocument {
    pub graph: Value,
    pub unused_node_ids: Vec<String>,
    unrecognized_nodes: Vec<UnrecognizedAuthoringNode>,
}

impl WorkflowGraph {
    /// Returns unused authoring nodes without requiring executable node configuration.
    ///
    /// The list includes unrecognized kinds that are also unreachable. Execution uses that
    /// combined set so a reference to a dropped node still fails closed. Callers that must
    /// tell a spare node from an unrecognized kind use [`Self::authoring_participation`].
    pub fn unused_node_ids(source: &str) -> Result<Vec<String>, GraphError> {
        Ok(project(source)?.unused_node_ids)
    }

    /// Splits spare nodes from kinds this version does not register.
    pub fn authoring_participation(source: &str) -> Result<AuthoringParticipation, GraphError> {
        let document = project(source)?;
        let unrecognized_ids: HashSet<&str> = document
            .unrecognized_nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect();
        Ok(AuthoringParticipation {
            unused_node_ids: document
                .unused_node_ids
                .into_iter()
                .filter(|id| !unrecognized_ids.contains(id.as_str()))
                .collect(),
            unrecognized_nodes: document.unrecognized_nodes,
        })
    }
}

/// Validates ownership before following edges, so filtering cannot hide corrupt boundaries.
pub(super) fn project(source: &str) -> Result<ExecutionDocument, GraphError> {
    let mut document: Value = serde_json::from_str(source).map_err(|_| GraphError::InvalidJson)?;
    if !document.is_object() {
        return Err(GraphError::InvalidJson);
    }
    let raw_nodes = document["nodes"]
        .as_array()
        .ok_or(GraphError::MissingNodes)?;
    let raw_edges = document["edges"]
        .as_array()
        .ok_or(GraphError::MissingEdges)?;
    let mut nodes = BTreeMap::new();
    let mut starts = HashMap::new();
    let mut unrecognized_nodes = Vec::new();
    for raw in raw_nodes {
        let id = required_string(&raw["id"], "missing id")?;
        let kind = required_string(&raw["data"]["kind"], &format!("node {id} has no node type"))?;
        // Known-but-unsupported kinds stay in the graph so parse can reject them by name.
        // A string with no variant is not a spare node the author parked off to the side.
        if NodeType::from_str(kind).is_err() {
            unrecognized_nodes.push(UnrecognizedAuthoringNode {
                node_id: id.to_string(),
                kind: kind.to_string(),
            });
        }
        let container = raw["data"]
            .get("containerId")
            .map(|value| required_string(value, "containerId must be a non-empty string"))
            .transpose()?;
        // React Flow permits null root parentage, but an explicit domain owner must be named.
        let parent = match raw.get("parentId") {
            None | Some(Value::Null) => None,
            Some(value) => Some(required_string(
                value,
                "parentId must be a non-empty string",
            )?),
        };
        if container.is_some() && parent.is_some() && container != parent {
            return Err(invalid("parentId disagrees with data.containerId"));
        }
        let owner = container.or(parent);
        if nodes.insert(id, Node { id, kind, owner }).is_some() {
            return Err(GraphError::DuplicateNodeId { node_id: id.into() });
        }
        if kind == "start" && starts.insert(owner, id).is_some() {
            return Err(GraphError::MultipleStartNodes);
        }
    }
    for raw in raw_nodes {
        let id = required_string(&raw["id"], "missing id")?;
        let node = &nodes[id];
        if let Some(owner) = node.owner {
            let Some(parent) = nodes.get(owner) else {
                return Err(invalid(&format!("unknown container owner {owner}")));
            };
            let expected = if raw["data"].get("containerId").is_some() {
                "loop"
            } else {
                "iteration"
            };
            if parent.kind != expected
                || parent.owner.is_some()
                || matches!(node.kind, "loop" | "iteration")
            {
                return Err(GraphError::InvalidRegion {
                    node_id: node.id.into(),
                    reason: "invalid or nested container ownership".into(),
                });
            }
        }
    }
    if nodes.values().any(|node| node.kind == "loop") && document["schemaVersion"] != 2 {
        return Err(invalid("container graphs require schemaVersion 2"));
    }
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in raw_edges {
        let source = required_string(&edge["source"], "edge has no source")?;
        let target = required_string(&edge["target"], "edge has no target")?;
        let from = nodes.get(source).ok_or_else(|| GraphError::DanglingEdge {
            node_id: source.into(),
        })?;
        let to = nodes.get(target).ok_or_else(|| GraphError::DanglingEdge {
            node_id: target.into(),
        })?;
        // Iteration entry edges are the only legal edges across containment boundaries.
        if from.owner != to.owner && !(from.kind == "iteration" && to.owner == Some(source)) {
            return Err(GraphError::InvalidRegion {
                node_id: source.into(),
                reason: "edge crosses a container boundary".into(),
            });
        }
        outgoing.entry(source).or_default().push(target);
    }
    let mut active = HashSet::new();
    let mut frontier = starts.get(&None).copied().into_iter().collect::<Vec<_>>();
    // A missing root entry is diagnosed by the existing engine validation. Keeping the
    // document intact here also preserves parse-time errors for malformed entryless graphs.
    if frontier.is_empty() {
        return Ok(ExecutionDocument {
            graph: document,
            unused_node_ids: Vec::new(),
            unrecognized_nodes,
        });
    }
    while let Some(id) = frontier.pop() {
        if !active.insert(id) {
            continue;
        }
        if nodes[id].kind == "loop" {
            let entry = starts
                .get(&Some(id))
                .ok_or_else(|| invalid("every active Loop requires one Start node"))?;
            frontier.push(entry);
        }
        if let Some(targets) = outgoing.get(id) {
            frontier.extend(targets);
        }
    }
    let unused_node_ids: Vec<_> = nodes
        .keys()
        .filter(|id| !active.contains(**id))
        .map(|id| (*id).to_string())
        .collect();
    let active: HashSet<String> = active.into_iter().map(str::to_string).collect();
    document["nodes"]
        .as_array_mut()
        .ok_or(GraphError::MissingNodes)?
        .retain(|node| node["id"].as_str().is_some_and(|id| active.contains(id)));
    document["edges"]
        .as_array_mut()
        .ok_or(GraphError::MissingEdges)?
        .retain(|edge| {
            edge["source"]
                .as_str()
                .is_some_and(|id| active.contains(id))
                && edge["target"]
                    .as_str()
                    .is_some_and(|id| active.contains(id))
        });
    Ok(ExecutionDocument {
        graph: document,
        unused_node_ids,
        unrecognized_nodes,
    })
}

/// Structural identity must be usable even when executable configuration is incomplete.
fn required_string<'a>(value: &'a Value, reason: &str) -> Result<&'a str, GraphError> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| invalid(reason))
}

/// Uses the existing typed graph error boundary for authoring structure failures.
fn invalid(reason: &str) -> GraphError {
    GraphError::InvalidNode {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests;

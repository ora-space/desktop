use crate::workflow_run::engine::node_type::{NodeType, UnknownNodeType};
use petgraph::Direction;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// A parsed workflow execution graph.
///
/// Deserializes a frozen React Flow document (the snapshot graph) into a validated DAG. The
/// graph is immutable after construction; every topology query is deterministic.
#[derive(Debug, Clone)]
pub struct WorkflowGraph {
    graph: DiGraph<WorkflowGraphNode, ()>,
    index_by_id: HashMap<String, NodeIndex>,
    /// Rank of each node in the unique `toposort` order, used to order transitive closures.
    topo_rank: HashMap<NodeIndex, usize>,
}

/// One node in a parsed workflow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowGraphNode {
    pub id: String,
    pub node_type: NodeType,
    pub title: String,
    pub description: String,
    /// Free-form instruction carried by `start`/`output` nodes (`data.instruction`).
    pub instruction: Option<String>,
    /// Execution contract of an `agent` node; absent for control nodes.
    pub agent_config: Option<AgentConfig>,
}

/// The executable contract of an `agent` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub executor: AgentExecutor,
    pub role_id: Option<String>,
    pub skills: Vec<AgentSkill>,
    pub prompt: String,
}

/// The agent CLI and model an `agent` node must run with.
///
/// `agent_cli` stays a string here; validating it as an agent identity and checking
/// runtime availability happens in the session driver (phase 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutor {
    pub agent_cli: String,
    pub model_id: String,
}

/// One skill an agent node declares; only `enabled` skills are materialized at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkill {
    pub skill_id: String,
    pub enabled: bool,
}

/// Structural failures discovered while deserializing and validating a frozen graph.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphError {
    #[error("workflow graph is not valid JSON")]
    InvalidJson,
    #[error("workflow graph is missing the nodes array")]
    MissingNodes,
    #[error("workflow graph is missing the edges array")]
    MissingEdges,
    #[error("invalid node: {reason}")]
    InvalidNode { reason: String },
    #[error("node {node_id} has unknown node type {value}")]
    UnknownNodeType { node_id: String, value: String },
    #[error("edge references missing node {node_id}")]
    DanglingEdge { node_id: String },
    #[error("workflow graph contains a cycle")]
    CycleDetected,
    #[error("workflow graph contains more than one start node")]
    MultipleStartNodes,
    #[error("duplicate node id: {node_id}")]
    DuplicateNodeId { node_id: String },
}

/// Wire shape of a frozen React Flow document.
///
/// Unknown top-level metadata fields (`id`, `name`, `description`, `updatedAt`, `viewport`) are
/// ignored by serde; only `nodes` and `edges` participate in execution.
#[derive(Debug, Deserialize)]
struct ReactFlowEnvelope {
    nodes: Option<Vec<WireNode>>,
    edges: Option<Vec<WireEdge>>,
}

/// Wire shape of one React Flow node. The renderer `type` is irrelevant to execution.
#[derive(Debug, Deserialize)]
struct WireNode {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    data: Option<WireNodeData>,
}

/// Wire shape of a node's `data` payload.
///
/// `kind` is the workflow node type on the wire; React Flow reserves the node-level `type` for
/// the renderer component, so the executable kind lives in `data`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireNodeData {
    #[serde(rename = "kind")]
    node_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    agent_config: Option<WireAgentConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentConfig {
    #[serde(default)]
    executor: Option<WireAgentExecutor>,
    #[serde(default)]
    role_id: Option<String>,
    #[serde(default)]
    skills: Vec<WireAgentSkill>,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentExecutor {
    #[serde(default)]
    agent_cli: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAgentSkill {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// Wire shape of one React Flow edge; the edge `id` is metadata and is ignored by serde.
#[derive(Debug, Deserialize)]
struct WireEdge {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    target: Option<String>,
}

impl WireAgentConfig {
    fn into_model(self) -> AgentConfig {
        AgentConfig {
            executor: AgentExecutor {
                agent_cli: self
                    .executor
                    .as_ref()
                    .and_then(|executor| executor.agent_cli.clone())
                    .unwrap_or_default(),
                model_id: self
                    .executor
                    .as_ref()
                    .and_then(|executor| executor.model_id.clone())
                    .unwrap_or_default(),
            },
            role_id: self.role_id,
            skills: self
                .skills
                .into_iter()
                .map(WireAgentSkill::into_model)
                .collect(),
            prompt: self.prompt.unwrap_or_default(),
        }
    }
}

impl WireAgentSkill {
    fn into_model(self) -> AgentSkill {
        AgentSkill {
            skill_id: self.skill_id.unwrap_or_default(),
            // Missing `enabled` defaults to false so skills are never materialized by surprise.
            enabled: self.enabled.unwrap_or(false),
        }
    }
}

impl WorkflowGraph {
    /// Parses a frozen React Flow graph JSON into a validated DAG.
    pub fn parse(source: &str) -> Result<Self, GraphError> {
        let envelope: ReactFlowEnvelope =
            serde_json::from_str(source).map_err(|_| GraphError::InvalidJson)?;
        let wire_nodes = envelope.nodes.ok_or(GraphError::MissingNodes)?;
        let wire_edges = envelope.edges.ok_or(GraphError::MissingEdges)?;

        let mut graph = DiGraph::<WorkflowGraphNode, ()>::new();
        let mut index_by_id = HashMap::new();
        for wire_node in wire_nodes {
            let id = wire_node.id.ok_or_else(|| GraphError::InvalidNode {
                reason: "missing id".into(),
            })?;
            if index_by_id.contains_key(&id) {
                return Err(GraphError::DuplicateNodeId { node_id: id });
            }
            let data = wire_node.data.ok_or_else(|| GraphError::InvalidNode {
                reason: format!("node {id} has no data"),
            })?;
            let node_type = data
                .node_type
                .as_deref()
                .ok_or_else(|| GraphError::InvalidNode {
                    reason: format!("node {id} has no node type"),
                })?
                .parse::<NodeType>()
                .map_err(|UnknownNodeType(value)| GraphError::UnknownNodeType {
                    node_id: id.clone(),
                    value,
                })?;
            let node = WorkflowGraphNode {
                id: id.clone(),
                node_type,
                title: data.title.unwrap_or_default(),
                description: data.description.unwrap_or_default(),
                instruction: data.instruction,
                agent_config: data.agent_config.map(WireAgentConfig::into_model),
            };
            let index = graph.add_node(node);
            index_by_id.insert(id, index);
        }

        for wire_edge in wire_edges {
            let WireEdge { source, target, .. } = wire_edge;
            let source = source.as_deref().ok_or_else(|| GraphError::DanglingEdge {
                node_id: target.clone().unwrap_or_default(),
            })?;
            let target = target.as_deref().ok_or_else(|| GraphError::DanglingEdge {
                node_id: source.to_string(),
            })?;
            let source_index =
                index_by_id
                    .get(source)
                    .copied()
                    .ok_or_else(|| GraphError::DanglingEdge {
                        node_id: source.to_string(),
                    })?;
            let target_index =
                index_by_id
                    .get(target)
                    .copied()
                    .ok_or_else(|| GraphError::DanglingEdge {
                        node_id: target.to_string(),
                    })?;
            graph.add_edge(source_index, target_index, ());
        }

        let start_count = graph
            .node_weights()
            .filter(|node| node.node_type == NodeType::Start)
            .count();
        if start_count > 1 {
            return Err(GraphError::MultipleStartNodes);
        }

        let topo_order = toposort(&graph, None).map_err(|_| GraphError::CycleDetected)?;
        let topo_rank = topo_order
            .into_iter()
            .enumerate()
            .map(|(rank, index)| (index, rank))
            .collect();
        Ok(Self {
            graph,
            index_by_id,
            topo_rank,
        })
    }

    /// Returns the unique start node, if the graph has one (parse guarantees at most one).
    pub fn start_node(&self) -> Option<&WorkflowGraphNode> {
        self.nodes().find(|node| node.node_type == NodeType::Start)
    }

    /// Returns the node with the given id, if present.
    pub fn node(&self, id: &str) -> Option<&WorkflowGraphNode> {
        self.index_by_id.get(id).map(|&index| &self.graph[index])
    }

    /// Iterates over every node in node-index (insertion) order.
    pub fn nodes(&self) -> impl Iterator<Item = &WorkflowGraphNode> {
        self.graph.node_weights()
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Returns the direct successors of `id` in deterministic adjacency order.
    pub fn successors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&index) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.graph
            .neighbors_directed(index, Direction::Outgoing)
            .map(|neighbor| &self.graph[neighbor])
            .collect()
    }

    /// Returns the direct predecessors of `id` in deterministic adjacency order.
    pub fn predecessors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&index) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.graph
            .neighbors_directed(index, Direction::Incoming)
            .map(|neighbor| &self.graph[neighbor])
            .collect()
    }

    /// Returns the transitive (downstream) closure of `id`, excluding the seed, in topological order.
    pub fn transitive_successors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&seed) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.order_by_topology(self.reachable_from(seed, Direction::Outgoing))
    }

    /// Returns the transitive (upstream) closure of `id`, excluding the seed, in topological order.
    pub fn transitive_predecessors(&self, id: &str) -> Vec<&WorkflowGraphNode> {
        let Some(&seed) = self.index_by_id.get(id) else {
            return Vec::new();
        };
        self.order_by_topology(self.reachable_from(seed, Direction::Incoming))
    }

    /// Returns nodes that are not yet completed and whose every direct predecessor is completed.
    ///
    /// A node with no direct predecessors (the start node) is vacuously ready.
    pub fn ready_set(&self, completed: &HashSet<&str>) -> Vec<&WorkflowGraphNode> {
        self.nodes()
            .filter(|node| !completed.contains(node.id.as_str()))
            .filter(|node| {
                self.predecessors(&node.id)
                    .iter()
                    .all(|predecessor| completed.contains(predecessor.id.as_str()))
            })
            .collect()
    }

    /// Returns the first node whose type v1 cannot execute, in node-index order.
    pub fn first_unsupported_node(&self) -> Option<&WorkflowGraphNode> {
        self.nodes().find(|node| !node.node_type.supported())
    }

    /// Returns the ids of nodes not reachable from the unique start node via directed edges.
    pub fn unreachable_from_start(&self) -> Vec<String> {
        let Some(start) = self.start_node() else {
            return self.nodes().map(|node| node.id.clone()).collect();
        };
        let reachable: HashSet<&str> = self
            .transitive_successors(&start.id)
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        self.nodes()
            .filter(|node| node.id != start.id && !reachable.contains(node.id.as_str()))
            .map(|node| node.id.clone())
            .collect()
    }

    /// Collects every node reachable from `seed` following `direction` edges, excluding the seed.
    fn reachable_from(&self, seed: NodeIndex, direction: Direction) -> HashSet<NodeIndex> {
        let mut reached = HashSet::new();
        let mut stack = vec![seed];
        while let Some(current) = stack.pop() {
            for edge in self.graph.edges_directed(current, direction) {
                let neighbor = match direction {
                    Direction::Outgoing => edge.target(),
                    Direction::Incoming => edge.source(),
                };
                if reached.insert(neighbor) {
                    stack.push(neighbor);
                }
            }
        }
        reached
    }

    /// Sorts a set of node indices into the graph's topological order (upstream first).
    fn order_by_topology(&self, indices: HashSet<NodeIndex>) -> Vec<&WorkflowGraphNode> {
        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_by_key(|index| self.topo_rank.get(index).copied().unwrap_or(usize::MAX));
        indices
            .into_iter()
            .map(|index| &self.graph[index])
            .collect()
    }
}

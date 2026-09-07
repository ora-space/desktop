use super::condition::ELSE_BRANCH_ID;
use super::graph::{WorkflowGraph, WorkflowGraphEdge, WorkflowGraphNode};
use super::node_type::NodeType;
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus};
use std::collections::{BTreeMap, HashMap};

/// The projected scheduling state of one workflow node.
///
/// Branch activation is derived, never persisted: given the frozen graph, the committed node-run
/// history, and internal Condition decisions, a restart can recompute which branch is active and
/// which nodes will never run without exposing scheduler state as workflow variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectedNodeState {
    /// At least one incoming edge has not been resolved by its source yet.
    NotReached,
    /// Every incoming edge resolved to inactive; this node and its successors never run.
    Inactive,
    /// Every incoming edge is resolved, at least one is active, and no node-run exists yet.
    Ready,
    /// A node-run is actively computing.
    Running,
    /// A node-run is parked at `Pending` awaiting interactive input.
    AwaitingInput,
    Succeeded,
    Failed,
    Cancelled,
}

/// The resolution of one graph edge, derived from its source node's projected state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeState {
    /// The source has not reached a terminal-projected state that resolves the edge.
    Pending,
    Active,
    Inactive,
    Failed,
}

/// Computes the projected state of every node from persisted facts, converging on the active
/// subgraph that branch-aware scheduling must dispatch.
pub struct BranchProjection<'a> {
    graph: &'a WorkflowGraph,
    condition_decisions: &'a BTreeMap<String, String>,
    states: HashMap<String, ProjectedNodeState>,
}

impl<'a> BranchProjection<'a> {
    /// Builds the projection from the frozen graph, node-runs, and internal branch decisions.
    pub fn new(
        graph: &'a WorkflowGraph,
        node_runs: &[WorkflowNodeRun],
        condition_decisions: &'a BTreeMap<String, String>,
    ) -> Self {
        let mut states = HashMap::new();
        for node_run in node_runs {
            let state = match node_run.status {
                WorkflowNodeStatus::Succeeded => ProjectedNodeState::Succeeded,
                WorkflowNodeStatus::Failed => ProjectedNodeState::Failed,
                WorkflowNodeStatus::Cancelled => ProjectedNodeState::Cancelled,
                WorkflowNodeStatus::Running => ProjectedNodeState::Running,
                WorkflowNodeStatus::Pending => ProjectedNodeState::AwaitingInput,
            };
            states.insert(node_run.node_id.clone(), state);
        }
        let mut projection = Self {
            graph,
            condition_decisions,
            states,
        };
        projection.resolve_projected_nodes();
        projection
    }

    /// Returns the projected state of one node; nodes with no resolved state are unreached.
    pub fn state(&self, node_id: &str) -> ProjectedNodeState {
        self.states
            .get(node_id)
            .copied()
            .unwrap_or(ProjectedNodeState::NotReached)
    }

    /// Returns the nodes that are ready to start in this scheduling wave, in graph order.
    pub fn ready_nodes(&self) -> Vec<&WorkflowGraphNode> {
        self.graph
            .nodes()
            .filter(|node| self.state(&node.id) == ProjectedNodeState::Ready)
            .collect()
    }

    /// Whether any node is still computing or parked awaiting interactive input.
    pub fn has_in_flight(&self) -> bool {
        self.states.values().any(|state| {
            matches!(
                state,
                ProjectedNodeState::Running | ProjectedNodeState::AwaitingInput
            )
        })
    }

    /// Repeatedly derives Ready/Inactive states until no node changes, so branch deactivation
    /// propagates downstream through each iteration.
    fn resolve_projected_nodes(&mut self) {
        loop {
            let mut changed = false;
            for node in self.graph.nodes() {
                if self.states.contains_key(&node.id) {
                    continue;
                }
                let computed = self.compute_projected_state(&node.id);
                if matches!(
                    computed,
                    ProjectedNodeState::Ready | ProjectedNodeState::Inactive
                ) && self.states.insert(node.id.clone(), computed) != Some(computed)
                {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Derives one node's projected state from the resolution of its incoming edges.
    fn compute_projected_state(&self, node_id: &str) -> ProjectedNodeState {
        let edges = self.graph.incoming_edges(node_id);
        if edges.is_empty() {
            // The start node has no incoming edges and begins ready.
            return ProjectedNodeState::Ready;
        }
        let mut any_active = false;
        for edge in &edges {
            match self.edge_state(edge) {
                EdgeState::Pending => return ProjectedNodeState::NotReached,
                EdgeState::Failed => return ProjectedNodeState::Failed,
                EdgeState::Active => any_active = true,
                EdgeState::Inactive => {}
            }
        }
        if any_active {
            ProjectedNodeState::Ready
        } else {
            ProjectedNodeState::Inactive
        }
    }

    /// Resolves one edge from its source node's projected state and internal branch decision.
    fn edge_state(&self, edge: &WorkflowGraphEdge) -> EdgeState {
        match self.states.get(&edge.source) {
            Some(ProjectedNodeState::Succeeded) => {
                let Some(source) = self.graph.node(&edge.source) else {
                    return EdgeState::Pending;
                };
                if source.node_type == NodeType::Condition {
                    if condition_edge_active(source.id.as_str(), edge, self.condition_decisions) {
                        EdgeState::Active
                    } else {
                        EdgeState::Inactive
                    }
                } else {
                    EdgeState::Active
                }
            }
            Some(ProjectedNodeState::Inactive | ProjectedNodeState::Cancelled) => {
                EdgeState::Inactive
            }
            Some(ProjectedNodeState::Failed) => EdgeState::Failed,
            _ => EdgeState::Pending,
        }
    }
}

/// Whether a Condition's outgoing edge matches the branch it selected, by comparing the edge's
/// source port against the committed `selected_branch_id`.
fn condition_edge_active(
    condition_id: &str,
    edge: &WorkflowGraphEdge,
    condition_decisions: &BTreeMap<String, String>,
) -> bool {
    let Some(selected) = condition_decisions.get(condition_id) else {
        return false;
    };
    match edge.source_handle.as_deref() {
        // An unlabeled condition edge defaults to the else branch so graphs that predate source
        // handles still route somewhere safe instead of dead-ending the downstream node.
        None => selected == ELSE_BRANCH_ID,
        Some(handle) => handle == selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_run::engine::condition::ELSE_BRANCH_ID;
    use pretty_assertions::assert_eq;

    /// start → condition → (approved → ok-output, else → no-output).
    fn branching_graph() -> &'static str {
        r#"{
            "nodes": [
                {"id":"start","data":{"kind":"start"}},
                {"id":"review","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"review"}}},
                {"id":"c","data":{"kind":"condition","cases":[
                    {"id":"approved","logic":"and","conditions":[
                        {"variableSelector":["review","structured_output","approved"],"operator":"is","value":true}
                    ]}
                ]}},
                {"id":"ok","data":{"kind":"output"}},
                {"id":"no","data":{"kind":"output"}}
            ],
            "edges": [
                {"source":"start","target":"review"},
                {"source":"review","target":"c"},
                {"source":"c","sourceHandle":"approved","target":"ok"},
                {"source":"c","sourceHandle":"else","target":"no"}
            ]
        }"#
    }

    fn node_run(node_id: &str, status: WorkflowNodeStatus) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            ora_domain::WorkflowNodeRunId::new(format!("node-{node_id}")),
            ora_domain::WorkflowRunId::new("run-1"),
            node_id,
            "agent",
            None,
            status,
            None,
            None,
            None,
            None,
            Some(1),
            None,
            ora_domain::AuditFields::new(1, 1, false),
        )
    }

    fn ready_ids(
        graph: &WorkflowGraph,
        node_runs: Vec<WorkflowNodeRun>,
        condition_decisions: &BTreeMap<String, String>,
    ) -> Vec<String> {
        BranchProjection::new(graph, &node_runs, condition_decisions)
            .ready_nodes()
            .iter()
            .map(|node| node.id.clone())
            .collect()
    }

    /// The approved branch activates only the matching successor; the else branch is inactive.
    #[test]
    fn selected_branch_activates_only_the_matching_successor() {
        let graph = WorkflowGraph::parse(branching_graph()).unwrap();
        let decisions = BTreeMap::from([("c".to_string(), "approved".to_string())]);
        let node_runs = vec![
            node_run("start", WorkflowNodeStatus::Succeeded),
            node_run("review", WorkflowNodeStatus::Succeeded),
            node_run("c", WorkflowNodeStatus::Succeeded),
        ];
        assert_eq!(ready_ids(&graph, node_runs.clone(), &decisions), vec!["ok"]);

        // The other branch wins when the condition selects else.
        let else_decisions = BTreeMap::from([("c".to_string(), ELSE_BRANCH_ID.to_string())]);
        assert_eq!(ready_ids(&graph, node_runs, &else_decisions), vec!["no"]);
    }

    /// A node with a resolved active edge and a resolved inactive edge is still ready (fan-in).
    #[test]
    fn fan_in_with_an_inactive_branch_still_becomes_ready() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"c","data":{"kind":"condition","cases":[]}},
                    {"id":"left","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"left"}}},
                    {"id":"right","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"right"}}},
                    {"id":"merge","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"merge"}}}
                ],
                "edges": [
                    {"source":"c","sourceHandle":"a","target":"left"},
                    {"source":"c","sourceHandle":"else","target":"right"},
                    {"source":"left","target":"merge"},
                    {"source":"right","target":"merge"}
                ]
            }"#,
        )
        .unwrap();
        let decisions = BTreeMap::from([("c".to_string(), "a".to_string())]);
        // Only the active left branch ran; the lost right branch has no node-run and is projected
        // inactive, which still resolves merge's incoming edges so merge becomes ready.
        let node_runs = vec![
            node_run("c", WorkflowNodeStatus::Succeeded),
            node_run("left", WorkflowNodeStatus::Succeeded),
        ];
        let projection = BranchProjection::new(&graph, &node_runs, &decisions);
        assert_eq!(projection.state("right"), ProjectedNodeState::Inactive);
        assert_eq!(ready_ids(&graph, node_runs, &decisions), vec!["merge"]);
    }

    /// A node whose every incoming edge is inactive is projected inactive and never runs.
    #[test]
    fn all_inactive_edges_project_the_node_inactive() {
        let graph = WorkflowGraph::parse(branching_graph()).unwrap();
        let decisions = BTreeMap::from([("c".to_string(), "approved".to_string())]);
        let node_runs = vec![
            node_run("start", WorkflowNodeStatus::Succeeded),
            node_run("review", WorkflowNodeStatus::Succeeded),
            node_run("c", WorkflowNodeStatus::Succeeded),
        ];
        let projection = BranchProjection::new(&graph, &node_runs, &decisions);
        assert_eq!(projection.state("ok"), ProjectedNodeState::Ready);
        assert_eq!(projection.state("no"), ProjectedNodeState::Inactive);
    }

    /// An unlabeled condition edge is treated as the else branch, so legacy graphs that predate
    /// source handles still route somewhere safe instead of dead-ending.
    #[test]
    fn unlabeled_condition_edge_follows_else() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"c","data":{"kind":"condition","cases":[]}},
                    {"id":"no","data":{"kind":"output"}}
                ],
                "edges": [{"source":"c","target":"no"}]
            }"#,
        )
        .unwrap();
        let mut decisions = BTreeMap::from([("c".to_string(), ELSE_BRANCH_ID.to_string())]);
        let node_runs = vec![node_run("c", WorkflowNodeStatus::Succeeded)];
        let projection = BranchProjection::new(&graph, &node_runs, &decisions);
        assert_eq!(projection.state("no"), ProjectedNodeState::Ready);

        // When the condition selects a named case instead, the unlabeled edge is inactive.
        decisions.insert("c".to_string(), "case-1".to_string());
        let projection = BranchProjection::new(&graph, &node_runs, &decisions);
        assert_eq!(projection.state("no"), ProjectedNodeState::Inactive);
    }

    /// While a condition is still running, its successors remain unreached.
    #[test]
    fn unresolved_condition_leaves_successors_unreached() {
        let graph = WorkflowGraph::parse(branching_graph()).unwrap();
        let decisions = BTreeMap::new();
        let node_runs = vec![
            node_run("start", WorkflowNodeStatus::Succeeded),
            node_run("review", WorkflowNodeStatus::Succeeded),
            node_run("c", WorkflowNodeStatus::Running),
        ];
        let projection = BranchProjection::new(&graph, &node_runs, &decisions);
        assert_eq!(projection.state("c"), ProjectedNodeState::Running);
        assert_eq!(projection.state("ok"), ProjectedNodeState::NotReached);
        assert!(projection.has_in_flight());
    }

    /// In a linear graph every edge is active, so scheduling matches the plain ready set.
    #[test]
    fn linear_graph_advances_like_the_plain_ready_set() {
        let graph = WorkflowGraph::parse(
            r#"{
                "nodes": [
                    {"id":"start","data":{"kind":"start"}},
                    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
                    {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
                    {"id":"out","data":{"kind":"output"}}
                ],
                "edges": [
                    {"source":"start","target":"a"},
                    {"source":"a","target":"b"},
                    {"source":"b","target":"out"}
                ]
            }"#,
        )
        .unwrap();
        let decisions = BTreeMap::new();
        assert_eq!(ready_ids(&graph, vec![], &decisions), vec!["start"]);
        assert_eq!(
            ready_ids(
                &graph,
                vec![node_run("start", WorkflowNodeStatus::Succeeded)],
                &decisions
            ),
            vec!["a"]
        );
        assert_eq!(
            ready_ids(
                &graph,
                vec![
                    node_run("start", WorkflowNodeStatus::Succeeded),
                    node_run("a", WorkflowNodeStatus::Succeeded),
                ],
                &decisions
            ),
            vec!["b"]
        );
    }
}

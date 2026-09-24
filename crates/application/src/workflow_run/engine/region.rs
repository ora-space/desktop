//! Graph-structure helpers for composite-region scheduling and resume.
//!
//! These judgments live outside `engine.rs` so the scheduling core stays free of extra
//! region-bookkeeping bulk; they still answer from persisted rows and graph topology only.

use crate::workflow_run::engine::graph::WorkflowGraph;
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::ports::FailurePropagation;
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus};
use std::collections::BTreeSet;

/// Whether a `Running` row of this wire node type stands for work in flight outside the
/// scheduler, and therefore must finish before the run can resume.
///
/// Container rows (Iteration, Loop) are `Running` only while their region or round rows do the
/// work, and those child rows block a resume on their own. A container parked by a run that
/// went terminal has nothing in flight; refusing to resume on its account would leave the run
/// stuck, because scheduling never advances a terminal run. An unknown type is treated as live
/// work, the conservative answer.
pub fn running_row_blocks_resume(node_type: &str) -> bool {
    !node_type
        .parse::<NodeType>()
        .is_ok_and(|node_type| matches!(node_type, NodeType::Iteration | NodeType::Loop))
}

/// Resolves how a failure of the node with the given id propagates, structurally: any failure
/// inside a composite region resolves to the owning composite node with `Composite` semantics
/// (the row fails, the run stays), and the owner's error strategy decides the node's fate at
/// settlement — `fail` fails the node and the run there, `continue` records the round and
/// advances (ADR "iteration composite runtime" D4, D6). This is a graph-structure judgment,
/// not a node-type branch in the scheduling core.
pub(super) fn region_failure_propagation(
    graph: &WorkflowGraph,
    node_id: &str,
) -> FailurePropagation {
    match graph.region_owner(node_id) {
        Some(_) => FailurePropagation::Composite,
        None => FailurePropagation::Run,
    }
}

/// Derives the round a composite node's region is currently executing, from the region's
/// persisted rows only (v1 serial execution; ADR "iteration composite runtime" D2).
pub(super) fn region_rows_round(
    graph: &WorkflowGraph,
    node_run: &WorkflowNodeRun,
    node_runs: &[WorkflowNodeRun],
) -> Option<u32> {
    let region = graph.region(&node_run.node_id)?;
    node_runs
        .iter()
        .filter(|row| row.iteration.is_some() && region.contains(&row.node_id))
        .filter_map(|row| row.iteration)
        .max()
}

/// The composite that would restart if this node-run is the resume trigger.
///
/// Iteration region members, Loop body nodes, and the composite's own row share one resume
/// unit: the owning composite. A Loop is never resumed inside an open round, whatever left the
/// round unfinished (a failed member, a cancelled run, or a retry wait the run abandoned).
/// Ordinary nodes have no unit owner.
pub fn resume_unit_owner_id(graph: &WorkflowGraph, node_id: &str) -> Option<String> {
    if graph.region(node_id).is_some() || graph.loop_body(node_id).is_some() {
        return Some(node_id.to_string());
    }
    graph
        .region_owner(node_id)
        .or_else(|| graph.loop_owner(node_id))
        .map(str::to_string)
}

/// Node ids that run inside the composite `owner` and so belong to its resume unit: the members
/// of an Iteration region or the nodes of a Loop body. Empty for an ordinary node.
pub fn resume_unit_member_ids(graph: &WorkflowGraph, owner: &str) -> Vec<String> {
    if let Some(region) = graph.region(owner) {
        return region.member_ids.to_vec();
    }
    graph
        .loop_body(owner)
        .map(|(_, body)| body.nodes().map(|node| node.id.clone()).collect())
        .unwrap_or_default()
}

/// Node ids `resume_from_failure` must soft-delete for the current live rows.
///
/// The resume unit for anything inside a region or Loop body is the owning composite node.
/// Partial in-loop resume is explicitly out of scope: a failed or cancelled region or Loop body
/// row, or a failed/cancelled composite row, restarts the loop from round 1 by clearing the
/// composite, every member of every round, and every outer descendant of the composite. A Loop
/// row the failed run left `Running` (its round held a wait the run abandoned) is cleared the
/// same way; the repository closes its round scopes with it.
pub fn resume_clear_node_ids(graph: &WorkflowGraph, node_runs: &[WorkflowNodeRun]) -> Vec<String> {
    let mut failed = BTreeSet::new();
    for node_run in node_runs {
        if matches!(
            node_run.status,
            WorkflowNodeStatus::Failed | WorkflowNodeStatus::Cancelled
        ) {
            failed.insert(node_run.node_id.clone());
        }
    }
    let mut to_clear = BTreeSet::new();
    for node_id in &failed {
        if let Some(owner) = resume_unit_owner_id(graph, node_id) {
            insert_composite_unit(graph, &owner, &mut to_clear);
        } else {
            to_clear.insert(node_id.clone());
            for successor in graph.transitive_successors(node_id) {
                to_clear.insert(successor.id.clone());
            }
        }
    }
    to_clear.into_iter().collect()
}

/// Inserts the composite, every region member or Loop body node, and every outer descendant.
fn insert_composite_unit(graph: &WorkflowGraph, owner: &str, to_clear: &mut BTreeSet<String>) {
    to_clear.insert(owner.to_string());
    to_clear.extend(resume_unit_member_ids(graph, owner));
    for successor in graph.transitive_successors(owner) {
        to_clear.insert(successor.id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::{resume_clear_node_ids, resume_unit_member_ids, resume_unit_owner_id};
    use crate::workflow_run::engine::graph::WorkflowGraph;
    use ora_domain::{
        AuditFields, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId,
        WorkflowScopeId,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// `start → loop → out` next to an outer agent `a`; the Loop body is `entry → writer`.
    fn loop_graph() -> WorkflowGraph {
        let agent = |id: &str, container: Option<&str>| {
            let mut node = json!({"id": id, "data": {"kind": "agent", "agentConfig": {
                "executor": {"agentCli": "c", "modelId": "m"}, "prompt": id
            }}});
            if let Some(container) = container {
                node["parentId"] = json!(container);
                node["data"]["containerId"] = json!(container);
            }
            node
        };
        WorkflowGraph::parse(
            &json!({
                "schemaVersion": 2,
                "nodes": [
                    {"id": "start", "data": {"kind": "start"}},
                    agent("a", None),
                    {"id": "loop", "data": {"kind": "loop", "loopConfig": {
                        "maxIterations": 2,
                        "variables": [],
                        "until": {"logic": "and", "conditions": [
                            {"variableSelector": ["writer", "output"], "operator": "equals", "value": "done"}
                        ]},
                        "outputs": [{"name": "result", "variableSelector": ["writer", "output"]}]
                    }}},
                    {"id": "entry", "parentId": "loop", "data": {"kind": "start", "containerId": "loop"}},
                    agent("writer", Some("loop")),
                    {"id": "out", "data": {"kind": "output"}}
                ],
                "edges": [
                    {"source": "start", "target": "a"},
                    {"source": "start", "target": "loop"},
                    {"source": "entry", "target": "writer"},
                    {"source": "loop", "target": "out"}
                ]
            })
            .to_string(),
        )
        .unwrap()
    }

    fn row(node_id: &str, status: WorkflowNodeStatus) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("{node_id}-run")),
            WorkflowRunId::new("run"),
            WorkflowScopeId::new("scope"),
            node_id,
            "agent",
            None,
            status,
            None,
            None,
            None,
            None,
            None,
            None,
            AuditFields::new(1, 1, false),
        )
    }

    /// A Loop body node belongs to the Loop's resume unit exactly like an Iteration region
    /// member belongs to its composite; an outer node has no unit owner.
    #[test]
    fn loop_body_nodes_resume_through_their_loop() {
        let graph = loop_graph();
        assert_eq!(
            ["writer", "entry", "loop", "a", "out"]
                .map(|node_id| resume_unit_owner_id(&graph, node_id)),
            [
                Some("loop".to_string()),
                Some("loop".to_string()),
                Some("loop".to_string()),
                None,
                None
            ]
        );
        let mut members = resume_unit_member_ids(&graph, "loop");
        members.sort();
        assert_eq!(members, vec!["entry".to_string(), "writer".to_string()]);
        assert_eq!(resume_unit_member_ids(&graph, "a"), Vec::<String>::new());
    }

    /// A cancelled Loop body row (an abandoned retry wait) clears the whole Loop unit even while
    /// the Loop row itself is still `Running`, so the rerun starts from round 1.
    #[test]
    fn a_cancelled_loop_body_row_clears_the_whole_loop_unit() {
        let graph = loop_graph();
        let rows = [
            row("start", WorkflowNodeStatus::Succeeded),
            row("a", WorkflowNodeStatus::Failed),
            row("loop", WorkflowNodeStatus::Running),
            row("entry", WorkflowNodeStatus::Succeeded),
            row("writer", WorkflowNodeStatus::Cancelled),
        ];
        assert_eq!(
            resume_clear_node_ids(&graph, &rows),
            ["a", "entry", "loop", "out", "writer"].map(str::to_string)
        );
    }
}

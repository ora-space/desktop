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
/// Region members and the composite's own row share one resume unit: the owning composite.
/// Ordinary nodes have no unit owner.
pub fn resume_unit_owner_id(graph: &WorkflowGraph, node_id: &str) -> Option<String> {
    if graph.region(node_id).is_some() {
        return Some(node_id.to_string());
    }
    graph.region_owner(node_id).map(str::to_string)
}

/// Node ids `resume_from_failure` must soft-delete for the current live rows.
///
/// The resume unit for anything inside a region is the owning composite node. Partial in-loop
/// resume is explicitly out of scope: a failed or cancelled region row, or a failed/cancelled
/// composite row, restarts the loop from round 1 by clearing the composite, every region member
/// of every round, and every outer descendant of the composite.
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

/// Inserts the composite, every region member, and every outer descendant.
fn insert_composite_unit(graph: &WorkflowGraph, owner: &str, to_clear: &mut BTreeSet<String>) {
    to_clear.insert(owner.to_string());
    if let Some(region) = graph.region(owner) {
        to_clear.extend(region.member_ids.iter().cloned());
    }
    for successor in graph.transitive_successors(owner) {
        to_clear.insert(successor.id.clone());
    }
}

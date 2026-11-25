//! Minimal top-level debug planning for the execution coordinator (#1429).
//!
//! Editing a root-scope node re-runs that node and its transitive downstream, stops the affected
//! in-flight attempts, and keeps independent branches. This is the smallest scope that proves the
//! stop → apply → re-dispatch path; #1430 refines Condition re-routing and join settlement, and
//! #1431/#1432 own the while/foreach container scopes.

use ora_application::WorkflowGraph;
use ora_contracts::{WorkflowDebugImpact, WorkflowDebugPreview};
use ora_db::workflow_debug::{DebugPlanningInput, DebugRejection, Transaction};
use ora_domain::WorkflowNodeStatus;

/// Plans a top-level edit: the target plus its transitive successors are the recompute scope.
pub(in crate::workflow::run) fn plan(
    input: &DebugPlanningInput,
) -> Result<WorkflowDebugImpact, DebugRejection> {
    // Container members are scoped by their round/item, not the root DAG; the container strategies
    // (#1431/#1432) own them. Refuse here rather than mis-scope a member as a top-level node.
    if input.target.container_node_run_id.is_some() {
        return Err(DebugRejection::InvalidTarget);
    }
    let affected = downstream_closure(&input.candidate, &input.target.node_id);
    let is_affected = |node_id: &str| affected.iter().any(|node| node == node_id);

    let mut stop = Vec::new();
    let mut invalidate = Vec::new();
    let mut keep = Vec::new();
    for attempt in &input.current_attempts {
        if is_affected(&attempt.node_id) {
            invalidate.push(attempt.id.to_string());
            // Only an in-flight attempt needs quiescing; a settled one is retired by the settle.
            if attempt.status == WorkflowNodeStatus::Running {
                stop.push(attempt.id.to_string());
            }
        } else {
            keep.push(attempt.id.to_string());
        }
    }
    Ok(WorkflowDebugImpact {
        keep,
        invalidate,
        stop,
        schedule: affected,
        predicted_branch: input.predicted_branch.clone(),
    })
}

/// The target node and every node reachable from it along control edges, in stable order.
fn downstream_closure(graph: &WorkflowGraph, target: &str) -> Vec<String> {
    let mut affected = vec![target.to_string()];
    let mut index = 0;
    while index < affected.len() {
        let node_id = affected[index].clone();
        for successor in graph.successors(&node_id) {
            if !affected.iter().any(|node| node == &successor.id) {
                affected.push(successor.id.clone());
            }
        }
        index += 1;
    }
    affected
}

/// Retires the invalidated attempts so the scheduler re-dispatch them under the new revision.
///
/// The first real dispatch assigns the new attempt id (the `workflow_debug_admit` trigger binds it
/// to the revision applied in the same transaction); the settle never pre-assigns attempt rows.
pub(in crate::workflow::run) fn settle(
    transaction: &Transaction<'_>,
    preview: &WorkflowDebugPreview,
) -> Result<(), ora_db::DatabaseError> {
    for id in &preview.impact.invalidate {
        transaction.execute(
            "UPDATE workflow_node_runs SET is_deleted=1 WHERE id=?1 AND is_deleted=0",
            [id],
        )?;
    }
    Ok(())
}

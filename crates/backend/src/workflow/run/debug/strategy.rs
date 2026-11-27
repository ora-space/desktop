//! Root DAG impact planning and settlement. A condition can retire a previously selected path
//! without authorizing it to run again. The scheduler derives skips from the new decision.

use ora_application::{BranchProjection, ProjectedNodeState, WorkflowGraph, WorkflowRunPayload};
use ora_contracts::{WorkflowDebugImpact, WorkflowDebugPreview};
use ora_db::workflow_debug::{DebugPlanningInput, DebugRejection, Transaction};
use ora_domain::{WorkflowNodeStatus, WorkflowRunStatus, WorkflowScopeStatus};
use std::collections::BTreeSet;

/// Plans a top-level edit: the target plus its transitive successors are the recompute scope.
pub(in crate::workflow::run) fn plan(
    input: &DebugPlanningInput,
) -> Result<(WorkflowDebugImpact, Vec<String>), DebugRejection> {
    // Container members are scoped by their round/item, not the root DAG; the container strategies
    // (#1431/#1432) own them. Refuse here rather than mis-scope a member as a top-level node.
    if input.target.container_node_run_id.is_some() {
        return Err(DebugRejection::InvalidTarget);
    }
    let target = &input.target.node_id;
    let affected = downstream_closure(&input.candidate, target);
    let schedule = reroute_schedule(input, &affected);
    let is_affected = |node_id: &str| affected.iter().any(|node| node == node_id);

    let mut stop = Vec::new();
    let mut invalidate = Vec::new();
    let mut keep = Vec::new();
    // A while member belongs to its owner's actual execution, not just its definition.
    let round_scopes: BTreeSet<_> = input
        .current_scopes
        .iter()
        .filter(|scope| {
            input.current_attempts.iter().any(|owner| {
                owner.id == scope.parent_loop_node_run_id
                    && owner.scope_id.as_ref() == input.target.scope_id
                    && is_affected(&owner.node_id)
            })
        })
        .map(|scope| &scope.id)
        .collect();
    for attempt in &input.current_attempts {
        let in_round = round_scopes.contains(&attempt.scope_id);
        if attempt.scope_id.as_ref() != input.target.scope_id && !in_round {
            continue;
        }
        if in_round
            || is_affected(&attempt.node_id)
            || input
                .candidate
                .region_owner(&attempt.node_id)
                .is_some_and(is_affected)
        {
            invalidate.push(attempt.id.to_string());
            // Parked attempts are isolated by the fence until retirement; only running ones need a stop.
            if attempt.status == WorkflowNodeStatus::Running {
                stop.push(attempt.id.to_string());
            }
        } else {
            keep.push(attempt.id.to_string());
        }
    }
    Ok((
        WorkflowDebugImpact {
            keep,
            invalidate,
            stop,
            schedule,
            predicted_branch: input.predicted_branch.clone(),
        },
        affected,
    ))
}

/// Re-projects the DAG after withdrawing affected attempts. A join on the discarded
/// branch can still receive an active edge from an independent predecessor.
fn reroute_schedule(input: &DebugPlanningInput, affected: &[String]) -> Vec<String> {
    let Some(branch) = &input.predicted_branch else {
        return affected.to_vec();
    };
    let graph = &input.candidate;
    let target = &input.target.node_id;
    let mut decisions = input
        .current_payload
        .as_ref()
        .map(WorkflowRunPayload::resolved_condition_decisions)
        .unwrap_or_default();
    decisions.insert(target.clone(), branch.clone());
    let kept = input
        .current_attempts
        .iter()
        .filter(|row| {
            row.scope_id.as_ref() == input.target.scope_id
                && row.iteration.is_none()
                && !affected.contains(&row.node_id)
        })
        .map(|row| (row.node_id.as_str(), row.status));
    let statuses = kept.chain(std::iter::once((
        target.as_str(),
        WorkflowNodeStatus::Succeeded,
    )));
    let projection = BranchProjection::from_outer_statuses(graph, statuses, &decisions);
    // Unresolved downstream Conditions can still select either branch at execution.
    affected
        .iter()
        .filter(|id| {
            projection.state(graph.region_owner(id).unwrap_or(id)) != ProjectedNodeState::Inactive
        })
        .cloned()
        .collect()
}

/// The target node and every node reachable from it along control edges, in stable order.
fn downstream_closure(graph: &WorkflowGraph, target: &str) -> Vec<String> {
    let mut affected = vec![target.to_string()];
    let mut index = 0;
    while index < affected.len() {
        let node_id = affected[index].clone();
        for edge in graph.outgoing_edges(&node_id) {
            if !affected.contains(&edge.target) {
                affected.push(edge.target);
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
    run_id: &str,
    now: i64,
    preview: &WorkflowDebugPreview,
) -> Result<(), ora_db::DatabaseError> {
    let mut retired = BTreeSet::new();
    let ids = serde_json::to_string(&preview.impact.invalidate)?;
    let mut writers = transaction.prepare("SELECT DISTINCT node_id FROM workflow_node_runs WHERE run_id=?1 AND id IN (SELECT value FROM json_each(?2))")?;
    for writer in writers.query_map([run_id, &ids], |r| r.get::<_, String>(0))? {
        retired.insert(writer?);
    }
    transaction.execute("UPDATE workflow_node_runs SET is_deleted=1,updated_at=?3 WHERE run_id=?1 AND is_deleted=0 AND id IN (SELECT value FROM json_each(?2))", (run_id,&ids,now))?;
    transaction.execute("UPDATE workflow_execution_scopes SET status=?4,updated_at=?3 WHERE run_id=?1 AND status IN (?5,?6) AND parent_loop_node_run_id IN (SELECT value FROM json_each(?2))", (run_id,&ids,now,WorkflowScopeStatus::Cancelled.database_value(),WorkflowScopeStatus::Pending.database_value(),WorkflowScopeStatus::Running.database_value()))?;
    if retired.is_empty() {
        return Ok(());
    }
    // A run that ended independently during quiescence keeps its terminal projection.
    // Terminal-sourced edits have already reopened their run in this transaction.
    let raw: Option<String> = transaction.query_row(
        "SELECT CASE WHEN run_status=?2 THEN payload END FROM workflow_runs WHERE id=?1",
        (run_id, WorkflowRunStatus::Running.database_value()),
        |row| row.get(0),
    )?;
    if let Some(raw) = raw {
        let mut payload: WorkflowRunPayload = serde_json::from_str(&raw)?;
        payload.withdraw_writers(&retired);
        transaction.execute(
            "UPDATE workflow_runs SET payload=?1 WHERE id=?2",
            [serde_json::to_string(&payload)?.as_str(), run_id],
        )?;
    }
    Ok(())
}

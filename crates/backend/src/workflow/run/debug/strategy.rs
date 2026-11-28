//! Root DAG impact planning and settlement. A condition can retire a previously selected path
//! without authorizing it to run again. The scheduler derives skips from the new decision.

use ora_application::{BranchProjection, ProjectedNodeState, WorkflowGraph, WorkflowRunPayload};
use ora_contracts::{WorkflowDebugImpact, WorkflowDebugPreview};
use ora_db::workflow_debug::{DebugPlanningInput, DebugRejection, Transaction};
use ora_domain::{WorkflowNodeStatus, WorkflowRunStatus, WorkflowScopeStatus};
use std::collections::BTreeSet;

mod foreach;
mod while_member;

/// Resolve the target scope before fencing every affected round.
pub(in crate::workflow::run) fn plan(
    input: &DebugPlanningInput,
) -> Result<(WorkflowDebugImpact, Vec<String>), DebugRejection> {
    let foreach =
        input.target.index.is_some() || input.candidate.region(&input.target.node_id).is_some();
    let (mut impact, mut fence) = match (foreach, input.target.round.is_some()) {
        (true, true) => return Err(DebugRejection::InvalidTarget),
        (true, false) => foreach::plan(input)?,
        (false, true) => {
            let impact = while_member::plan(input)?;
            let fence = impact.schedule.clone();
            (impact, fence)
        }
        (false, false) => plan_root(input)?,
    };
    // The edited round keeps its own Loop. Only rebuilt containers authorize their whole body.
    let retained = input.candidate.loop_owner(&input.target.node_id);
    for positions in [&mut impact.schedule, &mut fence] {
        let members: Vec<_> = positions
            .iter()
            .filter(|id| Some(id.as_str()) != retained)
            .filter_map(|id| input.candidate.loop_body(id))
            .flat_map(|(_, body)| body.nodes().map(|node| node.id.clone()))
            .collect();
        for id in members {
            if !positions.contains(&id) {
                positions.push(id);
            }
        }
    }
    for exit in input.current_attempts.iter().filter(|row| {
        row.node_type == "loopExit" && impact.invalidate.iter().any(|id| id == row.id.as_ref())
    }) {
        let scope = input
            .current_scopes
            .iter()
            .find(|scope| scope.id == exit.scope_id)
            .ok_or(DebugRejection::MissingInputEvidence)?;
        let state: ora_application::LoopRoundExecutionState =
            serde_json::from_str(&scope.state).map_err(|_| DebugRejection::MissingInputEvidence)?;
        let Some(request) = state
            .exit
            .filter(|request| request.node_run_id == exit.id.as_ref())
        else {
            continue;
        };
        match (request.cleanup_result, scope.status) {
            (Some(Ok(())), _) => {}
            // Before receipts were recorded, only successful settlement proved cleanup.
            (None, WorkflowScopeStatus::Succeeded) => {}
            (None, WorkflowScopeStatus::Pending | WorkflowScopeStatus::Running) => {
                return Err(DebugRejection::LoopExitCleanupPending);
            }
            (Some(Err(_)), _)
            | (None, WorkflowScopeStatus::Failed | WorkflowScopeStatus::Cancelled) => {
                return Err(DebugRejection::LoopExitRecoveryRequired);
            }
        }
    }
    Ok((impact, fence))
}

/// Resolve physical round scopes only for affected Loop executions in the target root.
fn rebuilt_rounds(
    input: &DebugPlanningInput,
    affected: impl Fn(&str) -> bool,
) -> BTreeSet<&ora_domain::WorkflowScopeId> {
    let root = input
        .current_attempts
        .iter()
        .find(|row| Some(row.id.as_ref()) == input.target.container_node_run_id.as_deref())
        .map_or(input.target.scope_id.as_str(), |owner| {
            owner.scope_id.as_ref()
        });
    let owners: BTreeSet<_> = input
        .current_attempts
        .iter()
        .filter(|row| {
            row.node_type == "loop" && row.scope_id.as_ref() == root && affected(&row.node_id)
        })
        .map(|row| &row.id)
        .collect();
    input
        .current_scopes
        .iter()
        .filter(|scope| owners.contains(&scope.parent_loop_node_run_id))
        .map(|scope| &scope.id)
        .collect()
}

/// Plan a root edit against both the old and newly selected routes.
fn plan_root(
    input: &DebugPlanningInput,
) -> Result<(WorkflowDebugImpact, Vec<String>), DebugRejection> {
    // Container members are scoped by their round/item, not the root DAG; the container strategies
    // (#1431/#1432) own them. Refuse here rather than mis-scope a member as a top-level node.
    if input.target.container_node_run_id.is_some() {
        return Err(DebugRejection::InvalidTarget);
    }
    if let Some((config, _)) = input.candidate.loop_body(&input.target.node_id) {
        // Only entrance values must exist now. Feedback and until expressions read
        // newly produced members later, when the scheduler completes the new round.
        config
            .initialize_carried(&input.pool)
            .map_err(|_| DebugRejection::MissingInputEvidence)?;
    }
    let target = &input.target.node_id;
    let affected = downstream_closure(&input.candidate, target);
    let schedule = reroute_schedule(input, &affected);
    let is_affected = |node_id: &str| affected.iter().any(|node| node == node_id);

    let mut stop = Vec::new();
    let mut invalidate = Vec::new();
    let mut keep = Vec::new();
    let round_scopes = rebuilt_rounds(input, is_affected);
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
            let local = attempt.node_type == "loop" || in_round && attempt.node_type != "agent";
            if attempt.status == WorkflowNodeStatus::Running && !local {
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
    let is_foreach = preview.target.index.is_some()
        || transaction.query_row(
            "SELECT node_type='iteration' FROM workflow_debug_previews WHERE id=?1",
            [&preview.preview_id],
            |row| row.get::<_, bool>(0),
        )?;
    if is_foreach {
        foreach::restore(transaction, preview)
            .map_err(|error| ora_db::DatabaseError::ForeachProjection(Box::new(error)))?;
    } else if preview.target.round.is_some() {
        ora_db::workflow_debug::restore_while_round(transaction, preview)?;
    }
    let mut retired = BTreeSet::new();
    let ids = serde_json::to_string(&preview.impact.invalidate)?;
    let mut writers = transaction.prepare("SELECT DISTINCT node_id FROM workflow_node_runs WHERE run_id=?1 AND id IN (SELECT value FROM json_each(?2))")?;
    for writer in writers.query_map([run_id, &ids], |r| r.get::<_, String>(0))? {
        retired.insert(writer?);
    }
    if preview.target.round.is_some() {
        retired.extend(preview.impact.schedule.iter().cloned());
    }
    transaction.execute("UPDATE workflow_node_runs SET is_deleted=1,updated_at=?3 WHERE run_id=?1 AND is_deleted=0 AND id IN (SELECT value FROM json_each(?2))", (run_id,&ids,now))?;
    transaction.execute("UPDATE workflow_execution_scopes SET status=?4,updated_at=?3 WHERE run_id=?1 AND status IN (?5,?6) AND parent_loop_node_run_id IN (SELECT value FROM json_each(?2))", (run_id,&ids,now,WorkflowScopeStatus::Cancelled.database_value(),WorkflowScopeStatus::Pending.database_value(),WorkflowScopeStatus::Running.database_value()))?;
    if is_foreach || retired.is_empty() {
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

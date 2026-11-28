//! Restore a selected member without changing the identity of the active Loop execution.

use super::*;
use ora_application::LoopRoundExecutionState;

/// Plan against the selected scope rather than all attempts with the same definition id.
pub(super) fn plan(input: &DebugPlanningInput) -> Result<WorkflowDebugImpact, DebugRejection> {
    let t = &input.target;
    let owner = input
        .candidate
        .loop_owner(&t.node_id)
        .ok_or(DebugRejection::InvalidTarget)?;
    let (config, body) = input
        .candidate
        .loop_body(owner)
        .ok_or(DebugRejection::InvalidTarget)?;
    // Carried variables belong to the Loop and cannot change within a round. A
    // dispatch record alone cannot prove that it used this round's entry values.
    let frozen: serde_json::Value = input.entry["scopeState"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .ok_or(DebugRejection::MissingInputEvidence)?;
    for variable in &config.variables {
        let key = format!("{owner}.{}", variable.name);
        let at_entry = frozen["variablePool"]["values"]
            .get(&key)
            .ok_or(DebugRejection::MissingInputEvidence)?;
        if input.pool.values.get(&key) != Some(at_entry) {
            return Err(DebugRejection::MissingInputEvidence);
        }
    }
    let scope = input
        .current_scopes
        .iter()
        .find(|s| {
            s.id.as_ref() == t.scope_id
                && Some(s.round_index) == t.round
                && Some(s.parent_loop_node_run_id.as_ref()) == t.container_node_run_id.as_deref()
        })
        .ok_or(DebugRejection::InvalidTarget)?;
    let later: std::collections::HashSet<_> = input
        .current_scopes
        .iter()
        .filter(|s| {
            s.parent_loop_node_run_id == scope.parent_loop_node_run_id
                && s.round_index > scope.round_index
        })
        .map(|s| &s.id)
        .collect();
    let inner = downstream_closure(body, &t.node_id);
    let state: LoopRoundExecutionState =
        serde_json::from_str(&scope.state).map_err(|_| DebugRejection::MissingInputEvidence)?;
    if let Some(exit) = &state.exit
        && !input.current_attempts.iter().any(|row| {
            row.id.as_ref() == exit.node_run_id
                && row.scope_id == scope.id
                && inner.contains(&row.node_id)
        })
    {
        // The selected edit cannot rewind an explicit exit on an independent path.
        return Err(DebugRejection::InvalidTarget);
    }
    let outer = downstream_closure(&input.candidate, owner);
    let mut impact = WorkflowDebugImpact {
        keep: vec![],
        invalidate: vec![],
        stop: vec![],
        schedule: inner.clone(),
        predicted_branch: input.predicted_branch.clone(),
    };
    impact.schedule.extend(outer.iter().cloned());
    let downstream = &outer[1..];
    let downstream_scopes = rebuilt_rounds(input, |node| downstream.iter().any(|id| id == node));
    for row in &input.current_attempts {
        let cancelled_by_exit = row.scope_id == scope.id
            && row.status == WorkflowNodeStatus::Cancelled
            && state.exit.as_ref().is_some_and(|exit| {
                exit.cancelled_attempts.as_ref().map_or_else(
                    // Earlier exits stored the common timestamp of the atomic cancellation.
                    || row.finished_at == Some(exit.requested_at),
                    |ids| ids.contains(row.id.as_ref()),
                )
            });
        if cancelled_by_exit {
            for node in downstream_closure(body, &row.node_id) {
                if !impact.schedule.contains(&node) {
                    impact.schedule.push(node);
                }
            }
        }
        if (row.scope_id == scope.id && inner.contains(&row.node_id))
            || later.contains(&row.scope_id)
            || downstream.contains(&row.node_id)
            || downstream_scopes.contains(&row.scope_id)
            || cancelled_by_exit
        {
            impact.invalidate.push(row.id.to_string());
            if row.status == WorkflowNodeStatus::Running && row.node_type == "agent" {
                impact.stop.push(row.id.to_string());
            }
        } else {
            impact.keep.push(row.id.to_string());
        }
    }
    Ok(impact)
}

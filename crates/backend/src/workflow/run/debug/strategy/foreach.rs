//! Item-scoped planning and transactional repair of the active foreach projection.
use ora_contracts::{WorkflowDebugImpact, WorkflowDebugPreview};
use ora_db::workflow_debug::{DebugPlanningInput, DebugRejection, Transaction, WorkflowDebugStore};
use ora_domain::WorkflowNodeStatus;
use std::collections::BTreeSet;

/// The owner instance stays live on a member edit, keeping its frozen inputs and the
/// configuration lineage of its members. An entrance edit instead retires the whole batch.
fn owner(input: &DebugPlanningInput) -> Result<&str, DebugRejection> {
    let node = &input.target.node_id;
    if let Some(index) = input.target.index {
        let owner = input
            .candidate
            .region_owner(node)
            .ok_or(DebugRejection::InvalidTarget)?;
        if !input.current_attempts.iter().any(|row| {
            Some(row.id.as_ref()) == input.target.container_node_run_id.as_deref()
                && row.node_id == owner
                && row.scope_id.as_ref() == input.target.scope_id
        }) {
            return Err(DebugRejection::InvalidTarget);
        }
        let values = &input.pool.values;
        if values
            .get(&format!("{owner}.index"))
            .and_then(serde_json::Value::as_u64)
            != Some(index.into())
            || !values.contains_key(&format!("{owner}.item"))
        {
            return Err(DebugRejection::MissingInputEvidence);
        }
        Ok(owner)
    } else {
        input
            .candidate
            .region(node)
            .ok_or(DebugRejection::InvalidTarget)?;
        Ok(node)
    }
}

/// Only the selected member's descendants are replayed within the target item. All members
/// of later items and all consumers of the batch projection become invalid.
pub(super) fn plan(input: &DebugPlanningInput) -> Result<WorkflowDebugImpact, DebugRejection> {
    let owner = owner(input)?;
    let graph = &input.candidate;
    let first = input.target.index;
    let target = &input.target.node_id;
    let tail: BTreeSet<_> = super::downstream_closure(graph, target)
        .into_iter()
        .collect();
    let after: BTreeSet<_> = super::downstream_closure(graph, owner)
        .into_iter()
        .collect();
    let mut payload = input
        .current_payload
        .clone()
        .ok_or(DebugRejection::MissingInputEvidence)?;
    payload
        .rewind_iteration_ledger(owner, first.unwrap_or_default())
        .map_err(|_| DebugRejection::MissingInputEvidence)?;
    let (mut keep, mut invalidate, mut stop) = (Vec::new(), Vec::new(), Vec::new());
    for row in &input.current_attempts {
        let affected = if row.scope_id.as_ref() != input.target.scope_id {
            false
        } else if graph.region_owner(&row.node_id) == Some(owner) {
            match (first, row.iteration) {
                (None, _) => true,
                (Some(start), Some(i)) => i > start || i == start && tail.contains(&row.node_id),
                _ => return Err(DebugRejection::MissingInputEvidence),
            }
        } else {
            after.contains(&row.node_id) && (row.node_id != owner || first.is_none())
        };
        if affected {
            invalidate.push(row.id.to_string());
            // Only agents have an external session. Composite and swift rows are
            // retired under the run gate, not sent to the session stopper.
            if row.status == WorkflowNodeStatus::Running && row.node_type == "agent" {
                stop.push(row.id.to_string());
            }
        } else {
            keep.push(row.id.to_string());
        }
    }
    let mut schedule = vec![target.clone()];
    schedule.extend(
        graph
            .nodes_in_topological_order()
            .into_iter()
            .filter(|n| n.id != *target && after.contains(&n.id))
            .map(|n| n.id.clone()),
    );
    Ok(WorkflowDebugImpact {
        keep,
        invalidate,
        stop,
        schedule,
        predicted_branch: input.predicted_branch.clone(),
    })
}

/// Restore only writers in the replayed region; a parallel branch may have completed
/// between the selected dispatch and this confirmation. Its current facts remain valid.
pub(super) fn restore(
    tx: &Transaction<'_>,
    preview: &WorkflowDebugPreview,
) -> Result<(), ora_db::DatabaseError> {
    let (run, input) = WorkflowDebugStore::confirmed_foreach_input(tx, preview)?;
    let owner = owner(&input)?;
    let graph = &input.candidate;
    let mut payload = input
        .current_payload
        .clone()
        .ok_or(DebugRejection::MissingInputEvidence)?;
    let first = input.target.index;
    payload
        .rewind_iteration_ledger(owner, first.unwrap_or_default())
        .map_err(|_| DebugRejection::MissingInputEvidence)?;
    let affected: BTreeSet<_> = super::downstream_closure(graph, owner)
        .into_iter()
        .collect();
    let member_tail: BTreeSet<_> = super::downstream_closure(graph, &input.target.node_id)
        .into_iter()
        .collect();
    for (key, decl) in &payload.variable_pool.catalog {
        if !affected.contains(&decl.writer) {
            continue;
        }
        // The target item can reuse its own active predecessor outputs, never later-item
        // values or the old target's output. Fresh containers have no reusable item facts.
        let reusable = first.is_some()
            && ((decl.writer == owner
                && (key == &format!("{owner}.item") || key == &format!("{owner}.index")))
                || graph.region_owner(&decl.writer) == Some(owner)
                    && !member_tail.contains(&decl.writer));
        if let Some(v) = input.pool.values.get(key).filter(|_| reusable) {
            payload.variable_pool.values.insert(key.clone(), v.clone());
        } else {
            payload.variable_pool.values.remove(key);
        }
    }
    payload.variable_pool.revision = payload.variable_pool.revision.saturating_add(1);
    payload
        .condition_decisions
        .retain(|n, _| !affected.contains(n));
    payload.iteration_condition_decisions.retain(|key, _| {
        let Some((node, i)) = key
            .rsplit_once('#')
            .and_then(|(n, i)| Some((n, i.parse::<u32>().ok()?)))
        else {
            return true;
        };
        if graph.region_owner(node) == Some(owner) {
            first.is_some_and(|start| i < start || i == start && !member_tail.contains(node))
        } else {
            !affected.contains(node)
        }
    });
    // Downstream foreach containers also need an empty effective ledger on re-entry.
    payload
        .iteration_ledger
        .retain(|node, _| node == owner || !affected.contains(node));
    tx.execute(
        "UPDATE workflow_runs SET payload=?2 WHERE id=?1",
        [&run, &serde_json::to_string(&payload)?],
    )?;
    if let Some(id) = &input.target.container_node_run_id {
        tx.execute("UPDATE workflow_node_runs SET status=1,output=NULL,error=NULL,finished_at=NULL WHERE id=?1 AND is_deleted=0", [id])?;
    }
    Ok(())
}

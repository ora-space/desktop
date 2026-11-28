//! Item-scoped planning and transactional repair of the active foreach projection.
use ora_application::RoundOutcome;
use ora_contracts::{WorkflowDebugImpact, WorkflowDebugPreview};
use ora_db::workflow_debug::{
    DebugPlanningInput, DebugRejection, ForeachReplay, Transaction, WorkflowDebugStore,
};
use ora_domain::{WorkflowNodeStatus, WorkflowRunStatus};
use serde_json::Value;
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

/// The item ledger can prove a typed collection even when the Agent's output column
/// holds only its final text. An unfinished item may have only the producer's row.
struct ItemReplay {
    tail: BTreeSet<String>,
    collection: Option<(String, Value)>,
}

impl ItemReplay {
    /// Resolve the collection before rewinding the ledger. Never borrow a later item's pool.
    fn from_input(input: &DebugPlanningInput, owner: &str) -> Result<Self, DebugRejection> {
        let graph = &input.candidate;
        let mut replay = Self {
            tail: super::downstream_closure(graph, &input.target.node_id)
                .into_iter()
                .collect(),
            collection: None,
        };
        let Some(index) = input.target.index else {
            return Ok(replay);
        };
        let config = graph
            .node(owner)
            .and_then(|n| n.iteration_config.as_ref())
            .ok_or(DebugRejection::InvalidTarget)?;
        let selector = &config.collect_selector;
        let producer = &selector.node_id;
        if replay.tail.contains(producer) {
            return Ok(replay);
        }
        let Some(row) = input.current_attempts.iter().find(|row| {
            row.scope_id.as_ref() == input.target.scope_id
                && row.iteration == Some(index)
                && row.node_id == *producer
                && row.status == WorkflowNodeStatus::Succeeded
        }) else {
            return Ok(replay);
        };
        let key = selector.qualified();
        let settled = input
            .current_payload
            .as_ref()
            .and_then(|p| p.iteration_ledger(owner))
            .and_then(|ledger| ledger.get(&index));
        let row_value = match (
            selector.root.as_str(),
            row.node_type.as_str(),
            row.output.as_ref(),
        ) {
            ("output", "agent", Some(text)) => Some(Value::String(text.clone())),
            ("output", "aggregator", Some(json)) => serde_json::from_str(json).ok(),
            _ => None,
        };
        let value = row_value
            .or_else(|| match settled {
                Some(RoundOutcome::Succeeded { output, .. }) => Some(output.clone()),
                _ => None,
            })
            .or_else(|| {
                let completed = input.entry["completedAttempts"].as_array()?;
                completed
                    .iter()
                    .any(|id| id.as_str() == Some(row.id.as_ref()))
                    .then(|| input.pool.values.get(&key).cloned())
                    .flatten()
            });
        if let Some(value) = value {
            replay.collection = Some((key, value));
        } else {
            replay
                .tail
                .extend(super::downstream_closure(graph, producer));
        }
        Ok(replay)
    }
}

/// The selected member's descendants and any unproven collector replay within the target item. All members
/// of later items and all consumers of the batch projection become invalid.
pub(super) fn plan(
    input: &DebugPlanningInput,
) -> Result<(WorkflowDebugImpact, Vec<String>), DebugRejection> {
    let owner = owner(input)?;
    let graph = &input.candidate;
    let first = input.target.index;
    let target = &input.target.node_id;
    let item = ItemReplay::from_input(input, owner)?;
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
    let rebuilt = super::rebuilt_rounds(input, |id| after.contains(id));
    for row in &input.current_attempts {
        let affected = if row.scope_id.as_ref() != input.target.scope_id {
            rebuilt.contains(&row.scope_id)
        } else if graph.region_owner(&row.node_id) == Some(owner) {
            match (first, row.iteration) {
                (None, _) => true,
                (Some(start), Some(i)) => {
                    i > start || i == start && item.tail.contains(&row.node_id)
                }
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
    let mut fence: BTreeSet<_> = schedule.iter().cloned().collect();
    if let Some(region) = graph.region(owner) {
        fence.extend(region.member_ids.iter().cloned());
    }
    Ok((
        WorkflowDebugImpact {
            keep,
            invalidate,
            stop,
            schedule,
            predicted_branch: input.predicted_branch.clone(),
        },
        fence.into_iter().collect(),
    ))
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
    let item = ItemReplay::from_input(&input, owner)?;
    // Stop and apply run in separate transactions. Keep the producer and its
    // descendants on exactly the side of the boundary the user confirmed.
    if first.is_some()
        && input.current_attempts.iter().any(|row| {
            row.scope_id.as_ref() == input.target.scope_id
                && row.iteration == first
                && graph.region_owner(&row.node_id) == Some(owner)
                && item.tail.contains(&row.node_id)
                    != preview.impact.invalidate.contains(&row.id.to_string())
        })
    {
        return Err(DebugRejection::MissingInputEvidence.into());
    }
    let member_tail = item.tail;
    // A terminal run keeps its visible variables and outcome. Its invalidated ledger
    // suffix still retires with the attempts and cannot contribute on later recovery.
    payload
        .iteration_ledger
        .retain(|node, _| node == owner || !affected.contains(node));
    if first.is_none() {
        let declarations = ora_application::WorkflowVariablePool::from_graph(graph).catalog;
        payload.variable_pool.catalog.extend(
            declarations
                .into_iter()
                .filter(|(_, decl)| affected.contains(&decl.writer)),
        );
    }
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
    let running: bool = tx.query_row(
        "SELECT run_status=?2 FROM workflow_runs WHERE id=?1",
        (&run, WorkflowRunStatus::Running.database_value()),
        |row| row.get(0),
    )?;
    let mut inputs = std::collections::BTreeMap::new();
    for (key, decl) in &payload.variable_pool.catalog {
        let reusable = first.is_some()
            && ((decl.writer == owner
                && (key == &format!("{owner}.item") || key == &format!("{owner}.index")))
                || graph.region_owner(&decl.writer) == Some(owner)
                    && !member_tail.contains(&decl.writer));
        if let Some(value) = input
            .pool
            .values
            .get(key)
            .filter(|_| reusable && affected.contains(&decl.writer))
        {
            inputs.insert(key.clone(), value.clone());
        }
    }
    inputs.extend(item.collection);
    let replay = ForeachReplay { affected, inputs };
    let container = input.target.container_node_run_id.as_deref();
    if running {
        replay.apply(tx, &mut payload, container)?;
    } else if let Some(container) = container {
        replay.park(tx, container)?;
    }
    tx.execute(
        "UPDATE workflow_runs SET payload=?2 WHERE id=?1",
        [&run, &serde_json::to_string(&payload)?],
    )?;
    Ok(())
}

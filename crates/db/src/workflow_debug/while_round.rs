//! Restore a selected while round inside the coordinator's apply transaction.
use super::{DebugRejection, Transaction};
use crate::DatabaseError;
use ora_application::LoopRoundExecutionState;
use ora_contracts::WorkflowDebugPreview;
use rusqlite::params;
use std::collections::HashSet;

/// Archive the affected slots even if a sibling terminated the run while its writers stopped.
pub fn restore_while_round(
    tx: &Transaction<'_>,
    preview: &WorkflowDebugPreview,
) -> Result<(), DatabaseError> {
    let id = &preview.target.scope_id;
    let owner = preview
        .target
        .container_node_run_id
        .as_deref()
        .ok_or(DebugRejection::InvalidTarget)?;
    let round = preview.target.round.ok_or(DebugRejection::InvalidTarget)?;
    tx.execute(
        "INSERT INTO workflow_debug_round_history(scope_id, operation_id, awaiting_replay)
         SELECT s.id, o.id, s.round_index > ?3 FROM workflow_execution_scopes s
         JOIN workflow_debug_operations o ON o.preview_id = ?1
         WHERE s.parent_loop_node_run_id = ?2 AND s.round_index >= ?3
           AND NOT EXISTS (SELECT 1 FROM workflow_debug_round_history h
                           WHERE h.scope_id = s.id AND h.awaiting_replay = 1)",
        params![preview.preview_id, owner, round],
    )?;
    let (state, running): (String, bool) = tx.query_row(
        "SELECT s.state, r.run_status = 1 FROM workflow_execution_scopes s
         JOIN workflow_runs r ON r.id = s.run_id WHERE s.id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !running {
        return Ok(());
    }
    tx.execute(
        "UPDATE workflow_execution_scopes SET status = 4
         WHERE parent_loop_node_run_id = ?1 AND round_index > ?2",
        params![owner, round],
    )?;
    let ids = serde_json::to_string(&preview.impact.invalidate)?;
    let mut query = tx.prepare(
        "SELECT node_id FROM workflow_node_runs
         WHERE scope_id = ?1 AND id IN (SELECT value FROM json_each(?2))",
    )?;
    let writers = query
        .query_map(params![id, ids], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    let mut state: LoopRoundExecutionState = serde_json::from_str(&state)?;
    state.variable_pool.values.retain(|key, _| {
        state
            .variable_pool
            .catalog
            .get(key)
            .is_none_or(|def| !writers.contains(&def.writer))
    });
    state
        .condition_decisions
        .retain(|key, _| !writers.contains(key));
    state.variable_pool.revision = state.variable_pool.revision.saturating_add(1);
    if state
        .exit
        .as_ref()
        .is_some_and(|exit| preview.impact.invalidate.contains(&exit.node_run_id))
    {
        state.exit = None;
    }
    tx.execute(
        "UPDATE workflow_execution_scopes SET state = ?2, status = 1 WHERE id = ?1",
        params![id, serde_json::to_string(&state)?],
    )?;
    // Keep the owner's entrance checkpoint when removing the old completion reason.
    tx.execute(
        "UPDATE workflow_node_runs SET status = 1, output = NULL, error = NULL, finished_at = NULL,
         payload = json_remove(payload, '$.stop_reason') WHERE id = ?1",
        [owner],
    )?;
    tx.execute(
        "UPDATE workflow_runs AS r SET state = json_set(COALESCE(r.state,'{}'), '$.current_nodes', (
             SELECT json_group_array(node_id) FROM (
                 SELECT a.value AS node_id FROM json_each(r.state, '$.current_nodes') a
                 WHERE NOT EXISTS (SELECT 1 FROM workflow_node_runs n JOIN json_each(?2) retired ON retired.value = n.id
                                   WHERE n.scope_id = owner.scope_id AND n.node_id = a.value)
                 UNION ALL SELECT owner.node_id WHERE NOT EXISTS (
                     SELECT 1 FROM json_each(r.state, '$.current_nodes') a WHERE a.value = owner.node_id))))
         FROM workflow_node_runs owner WHERE owner.id = ?1 AND r.id = owner.run_id",
        params![owner, ids],
    )?;
    Ok(())
}

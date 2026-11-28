//! Restore a selected while round inside the coordinator's apply transaction.
//!
//! A run can end independently while the edited round's writers stop (the "reverse race"). The
//! revision still applies, but the terminal run keeps its projection and dispatches nothing. The
//! round reopen is then parked on the Loop owner and performed by whichever entry point next makes
//! the run schedulable: ordinary failure recovery, or a later debug edit reopening the run.
//! Reopening there, instead of leaving the old slots active, keeps a later round from continuing
//! with the carried value of a retired attempt (ADR D4, invariant 3). The parked record also
//! carries the root-pool withdrawal the running path performs at apply (the Loop's exports and
//! its downstream), so nothing outside the Loop reads a retired result either.
use super::{DebugRejection, Transaction, WorkflowDebugImpact};
use crate::DatabaseError;
use ora_application::{LoopRoundExecutionState, WorkflowRunPayload};
use ora_contracts::WorkflowDebugPreview;
use ora_logging::ora_warn;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

/// Owner-row payload key holding round reopens parked by a terminal run.
const PARKED: &str = "$.debugWhileReplays";

/// One confirmed round reopen: the round to replay, the attempts whose results it withdraws
/// inside the round, and the writers whose root-pool projections it withdraws.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WhileReplay {
    operation_id: i64,
    scope_id: String,
    round: u32,
    invalidate: Vec<String>,
    schedule: Vec<String>,
    #[serde(default)]
    withdraw: BTreeSet<String>,
}

/// Archive the affected slots even if a sibling terminated the run while its writers stopped.
///
/// `withdraw` names the writers whose root-pool projections this edit retires. A running run
/// withdraws them in the caller's same transaction; a terminal run parks them with the round.
pub fn restore_while_round(
    tx: &Transaction<'_>,
    preview: &WorkflowDebugPreview,
    withdraw: &BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let owner = preview
        .target
        .container_node_run_id
        .as_deref()
        .ok_or(DebugRejection::InvalidTarget)?;
    let round = preview.target.round.ok_or(DebugRejection::InvalidTarget)?;
    let operation_id: i64 = tx.query_row(
        "SELECT id FROM workflow_debug_operations WHERE preview_id = ?1",
        [&preview.preview_id],
        |row| row.get(0),
    )?;
    // Attribute every affected slot to this operation. A slot only starts awaiting replay when
    // the reopen below actually archives it; one still archived by an earlier edit keeps that
    // owner until it is replayed.
    tx.execute(
        "INSERT INTO workflow_debug_round_history(scope_id, operation_id, awaiting_replay)
         SELECT s.id, ?3, 0 FROM workflow_execution_scopes s
         WHERE s.parent_loop_node_run_id = ?1 AND s.round_index >= ?2
           AND NOT EXISTS (SELECT 1 FROM workflow_debug_round_history h
                           WHERE h.scope_id = s.id AND h.awaiting_replay = 1)",
        params![owner, round, operation_id],
    )?;
    let replay = WhileReplay {
        operation_id,
        scope_id: preview.target.scope_id.clone(),
        round,
        invalidate: preview.impact.invalidate.clone(),
        schedule: preview.impact.schedule.clone(),
        withdraw: withdraw.clone(),
    };
    let running: bool = tx.query_row(
        "SELECT r.run_status = 1 FROM workflow_runs r
         JOIN workflow_node_runs owner ON owner.run_id = r.id WHERE owner.id = ?1",
        [owner],
        |row| row.get(0),
    )?;
    if running {
        return replay.reopen(tx, owner);
    }
    // The terminal projection stays as the sibling left it; recovery performs the reopen.
    tx.execute(
        &format!(
            "UPDATE workflow_node_runs SET payload = json_set(COALESCE(payload, '{{}}'), '{PARKED}',
                 json_insert(COALESCE(json_extract(payload, '{PARKED}'), json('[]')), '$[#]', json(?2)))
             WHERE id = ?1 AND is_deleted = 0"
        ),
        params![owner, serde_json::to_string(&replay)?],
    )?;
    Ok(())
}

/// Reopen round reopens parked on live Loop owners before the run schedules again. Owners in
/// `retiring` are being rebuilt by a wider scope and restart from their own entrance instead.
pub(crate) fn resume_while_replays(
    tx: &Transaction<'_>,
    run: &str,
    retiring: &[String],
) -> Result<(), DatabaseError> {
    let parked = parked_replays(tx, run, retiring)?;
    for (owner, replays) in parked {
        for replay in replays {
            if !replay.is_current(tx, &owner)? {
                // The Loop advanced without this reopen (for example under a build that did
                // not know the record). Reopening now would rewind settled rounds whose slots
                // carry no history for this operation.
                ora_warn!(
                    run_id = %run,
                    loop_node_run_id = %owner,
                    operation_id = replay.operation_id,
                    round = replay.round,
                    "dropping a parked while round reopen that no longer matches its Loop"
                );
                continue;
            }
            replay.reopen(tx, &owner)?;
            let raw: String = tx.query_row(
                "SELECT payload FROM workflow_runs WHERE id = ?1",
                [run],
                |row| row.get(0),
            )?;
            let mut payload: WorkflowRunPayload = serde_json::from_str(&raw)?;
            payload.withdraw_writers(&replay.withdraw);
            tx.execute(
                "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1",
                params![run, serde_json::to_string(&payload)?],
            )?;
        }
        tx.execute(
            &format!(
                "UPDATE workflow_node_runs SET payload = json_remove(payload, '{PARKED}') WHERE id = ?1"
            ),
            [&owner],
        )?;
    }
    Ok(())
}

/// A terminal edit that reopens the run also performs the parked round reopens. List their
/// positions in its schedule and read set, without changing which attempts this edit stops.
pub(super) fn include_parked_schedule(
    tx: &Transaction<'_>,
    run: &str,
    impact: &mut WorkflowDebugImpact,
    reads: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    for (_, replays) in parked_replays(tx, run, &impact.invalidate)? {
        for node in replays.into_iter().flat_map(|replay| replay.schedule) {
            reads.insert(node.clone());
            if !impact.schedule.contains(&node) {
                impact.schedule.push(node);
            }
        }
    }
    Ok(())
}

/// Parked reopens per live Loop owner of the run, oldest first, excluding `retiring` owners.
fn parked_replays(
    tx: &Transaction<'_>,
    run: &str,
    retiring: &[String],
) -> Result<Vec<(String, Vec<WhileReplay>)>, DatabaseError> {
    let mut statement = tx.prepare(&format!(
        "SELECT id, json_extract(payload, '{PARKED}') FROM workflow_node_runs
         WHERE run_id = ?1 AND node_type = 'loop' AND is_deleted = 0
           AND json_type(payload, '{PARKED}') = 'array'
           AND id NOT IN (SELECT value FROM json_each(?2)) ORDER BY id"
    ))?;
    let rows = statement
        .query_map(params![run, serde_json::to_string(retiring)?], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(owner, raw)| Ok((owner, serde_json::from_str(&raw)?)))
        .collect()
}

impl WhileReplay {
    /// Whether the Loop still stands where this edit left it: the edited round is the owner's
    /// scope at that index, and every later slot either has this edit's history row or is still
    /// archived, awaiting replay, by an earlier edit. The edit wrote a row for every later slot
    /// except those (`restore_while_round` leaves an archived slot with its first owner), and this
    /// build creates no slot between apply and consumption. A later slot with neither marker was
    /// created or replayed by another build, so the Loop moved on without this reopen.
    ///
    /// "Earlier" compares operation ids: they are assigned at acceptance, and the fence admits
    /// one stopping operation per run at a time, so ids order a run's operations exactly as
    /// their revisions do. How far the Loop had advanced before the edit does not matter.
    fn is_current(&self, tx: &Transaction<'_>, owner: &str) -> Result<bool, DatabaseError> {
        Ok(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_execution_scopes s
                 WHERE s.id = ?2 AND s.parent_loop_node_run_id = ?1 AND s.round_index = ?3
                   AND NOT EXISTS (SELECT 1 FROM workflow_execution_scopes later
                       WHERE later.parent_loop_node_run_id = ?1 AND later.round_index > ?3
                         AND NOT EXISTS (SELECT 1 FROM workflow_debug_round_history h
                                         WHERE h.scope_id = later.id AND (h.operation_id = ?4
                                           OR (h.operation_id < ?4 AND h.awaiting_replay = 1
                                               AND later.status = 4)))))",
            params![owner, self.scope_id, self.round, self.operation_id],
            |row| row.get(0),
        )?)
    }

    /// Archive the later slots, reopen the selected round without the retired results, and put
    /// the Loop owner back to running so the scheduler dispatches the round's new attempts.
    fn reopen(&self, tx: &Transaction<'_>, owner: &str) -> Result<(), DatabaseError> {
        tx.execute(
            "UPDATE workflow_execution_scopes SET status = 4
             WHERE parent_loop_node_run_id = ?1 AND round_index > ?2",
            params![owner, self.round],
        )?;
        tx.execute(
            "UPDATE workflow_debug_round_history AS h SET awaiting_replay = 1
             FROM workflow_execution_scopes s
             WHERE s.id = h.scope_id AND h.operation_id = ?3
               AND s.parent_loop_node_run_id = ?1 AND s.round_index > ?2
               AND NOT EXISTS (SELECT 1 FROM workflow_debug_round_history other
                               WHERE other.scope_id = h.scope_id AND other.awaiting_replay = 1)",
            params![owner, self.round, self.operation_id],
        )?;
        let id = &self.scope_id;
        let state: String = tx.query_row(
            "SELECT state FROM workflow_execution_scopes WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        let ids = serde_json::to_string(&self.invalidate)?;
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
            .is_some_and(|exit| self.invalidate.contains(&exit.node_run_id))
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
}

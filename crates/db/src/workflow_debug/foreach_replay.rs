//! Member inputs retained on the physical foreach owner until explicit terminal recovery.
use super::*;
use ora_application::WorkflowRunPayload;
use ora_domain::WorkflowNodeStatus;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The selected item's proven inputs and writers whose previous outputs are retiring.
#[derive(Serialize, Deserialize)]
pub struct ForeachReplay {
    pub affected: BTreeSet<String>,
    pub inputs: BTreeMap<String, Value>,
}

impl ForeachReplay {
    /// Keeps a terminal run's visible pool and the owner's terminal status until recovery.
    pub fn park(&self, tx: &Transaction<'_>, owner: &str) -> Result<(), DatabaseError> {
        tx.execute("UPDATE workflow_node_runs SET payload=json_set(COALESCE(payload,'{}'),'$.debugForeachReplay',json(?2))
                    WHERE id=?1 AND is_deleted=0", params![owner,serde_json::to_string(self)?])?;
        Ok(())
    }

    /// Installs only affected values. Independent writers may have advanced after the preview.
    pub fn apply(
        self,
        tx: &Transaction<'_>,
        payload: &mut WorkflowRunPayload,
        owner: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let pool = &mut payload.variable_pool;
        pool.values.retain(|key, _| {
            !pool
                .catalog
                .get(key)
                .is_some_and(|d| self.affected.contains(&d.writer))
        });
        pool.values.extend(self.inputs);
        pool.revision = pool.revision.saturating_add(1);
        payload
            .condition_decisions
            .retain(|id, _| !self.affected.contains(id));
        if let Some(id) = owner {
            tx.execute("UPDATE workflow_node_runs SET status=?2,output=NULL,error=NULL,finished_at=NULL,
                         payload=json_remove(payload,'$.debugForeachReplay') WHERE id=?1 AND is_deleted=0",
                       params![id, WorkflowNodeStatus::Running.database_value()])?;
        }
        Ok(())
    }
}

/// A terminal edit also reactivates previously confirmed member replays. Show that work in
/// its schedule and validate it at claim, without changing which attempts this edit stops.
pub(super) fn include_parked_schedule(
    tx: &Transaction<'_>,
    run: &str,
    impact: &mut WorkflowDebugImpact,
    reads: &mut BTreeSet<String>,
) -> Result<(), DatabaseError> {
    let mut stmt = tx.prepare(
        "SELECT DISTINCT node.value FROM workflow_node_runs n
        JOIN json_each(n.payload,'$.debugForeachReplay.affected') node
        WHERE n.run_id=?1 AND n.node_type='iteration' AND n.is_deleted=0
          AND n.id NOT IN (SELECT value FROM json_each(?2)) ORDER BY node.value",
    )?;
    let nodes = stmt.query_map(
        params![run, serde_json::to_string(&impact.invalidate)?],
        |r| r.get::<_, String>(0),
    )?;
    for node in nodes {
        let node = node?;
        reads.insert(node.clone());
        if !impact.schedule.contains(&node) {
            impact.schedule.push(node);
        }
    }
    Ok(())
}

/// Activate surviving owners before the scheduler sees the run as running. Owners cleared by a
/// wider recovery have already been retired and their replacement starts from its own entrance.
pub(crate) fn resume_foreach_replays(
    tx: &Transaction<'_>,
    run: &str,
    retiring: &[String],
) -> Result<(), DatabaseError> {
    let mut stmt = tx.prepare("SELECT id,json_extract(payload,'$.debugForeachReplay') FROM workflow_node_runs
        WHERE run_id=?1 AND node_type='iteration' AND is_deleted=0 AND json_type(payload,'$.debugForeachReplay')='object'
          AND id NOT IN (SELECT value FROM json_each(?2))")?;
    let owners = stmt
        .query_map(params![run, serde_json::to_string(retiring)?], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if owners.is_empty() {
        return Ok(());
    }
    let raw: String = tx.query_row(
        "SELECT payload FROM workflow_runs WHERE id=?1",
        [run],
        |r| r.get(0),
    )?;
    let mut payload = serde_json::from_str(&raw)?;
    for (id, raw) in owners {
        let replay: ForeachReplay = serde_json::from_str(&raw)?;
        replay.apply(tx, &mut payload, Some(&id))?;
    }
    tx.execute(
        "UPDATE workflow_runs SET payload=?2 WHERE id=?1",
        params![run, serde_json::to_string(&payload)?],
    )?;
    Ok(())
}

use super::*;
use ora_domain::WorkflowNodeRunId;

/// Selects run `?1`'s container owners (Loop or Iteration) that an applied debug edit reopened
/// and that have not dispatched anything since; see [`WorkflowDebugStore::reopened_containers`].
/// The boot failure path embeds the same query, so the exemption cannot drift between the two.
///
/// "Since" is anchored on revisions: every attempt is bound at admission to the run revision then
/// in effect and to its container (`workflow_debug_attempts`). A child at or above the newest
/// reopening edit's revision was dispatched after that reopen, and a pending or running child is
/// work in flight; either one makes the container ordinary interrupted work again (while D6).
pub(crate) const REOPENED_CONTAINERS: &str = "SELECT owner.id FROM workflow_node_runs owner
    JOIN (SELECT json_extract(p.plan, '$.target.containerNodeRunId') AS owner_id,
                 MAX(o.revision) AS revision
          FROM workflow_debug_operations o JOIN workflow_debug_previews p ON p.id = o.preview_id
          WHERE o.run_id = ?1 AND o.state = 'applied' GROUP BY 1) reopened
      ON reopened.owner_id = owner.id
    WHERE owner.run_id = ?1 AND owner.node_type IN ('loop', 'iteration') AND owner.status = 1
      AND owner.is_deleted = 0
      AND NOT EXISTS (SELECT 1 FROM workflow_debug_attempts a
                      JOIN workflow_node_runs child ON child.id = a.node_run_id
                      WHERE a.container_attempt = owner.id AND child.is_deleted = 0
                        AND (child.status IN (0, 1) OR a.revision >= reopened.revision))";

/// Performs the root-pool withdrawal earlier edits parked on a terminal run, before a debug
/// edit reopens that run and its scheduler reads the pool.
pub(crate) fn resume_parked_withdrawals(
    tx: &Transaction<'_>,
    run: &str,
) -> Result<(), DatabaseError> {
    let raw: Option<String> = tx.query_row(
        "SELECT payload FROM workflow_runs WHERE id=?1",
        [run],
        |r| r.get(0),
    )?;
    let Some(raw) = raw else { return Ok(()) };
    let mut payload: ora_application::WorkflowRunPayload = serde_json::from_str(&raw)?;
    if payload.withdraw_parked() {
        tx.execute(
            "UPDATE workflow_runs SET payload=?2 WHERE id=?1",
            params![run, serde_json::to_string(&payload)?],
        )?;
    }
    Ok(())
}

impl WorkflowDebugStore {
    /// A parked independent node is not a reason to skip resuming committed debug work at boot.
    pub fn has_dispatch(&self, run: &str) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM workflow_debug_dispatch WHERE run_id=?1)",
                [run],
                |r| r.get(0),
            )?)
        })
    }
    /// The previous Desktop driver is gone, but an external writer may have outlived it. Only a
    /// retry still in persisted backoff proves that no session was dispatched for that attempt.
    pub fn recover_stopping(
        &self,
        run: &str,
        now: i64,
        settle: impl FnOnce(&Transaction<'_>, &WorkflowDebugPreview) -> Result<(), DatabaseError>,
    ) -> Result<(), DatabaseError> {
        let intent = self.pool.with_connection(|db| {
            let Some((key,plan)) = db.query_row("SELECT o.request_id,p.plan FROM workflow_debug_operations o JOIN workflow_debug_previews p ON p.id=o.preview_id WHERE o.run_id=?1 AND o.state='stopping'",[run],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()? else {return Ok(None)};
            let preview: WorkflowDebugPreview = serde_json::from_str(&plan)?;
            let mut unsettled = Vec::new();
            for id in preview.impact.stop {
                let waiting: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM workflow_node_runs WHERE run_id=?1 AND id=?2 AND is_deleted=0 AND started_at IS NULL AND session_id IS NULL AND json_type(payload,'$.retry_wait')='object')",params![run,id],|r|r.get(0))?;
                if !waiting { unsettled.push(WorkflowNodeRunId::new(id)); }
            }
            Ok(Some((key,unsettled)))
        })?;
        if let Some((key, unsettled)) = intent {
            if unsettled.is_empty() {
                self.apply_after_stop(run, &key, now, settle)?;
            } else {
                self.record_stop_failure(
                    run,
                    &key,
                    "external isolation not proven after restart",
                    &unsettled,
                )?;
            }
        }
        Ok(())
    }

    /// Container owners whose round or item an applied edit reopened and whose new attempts
    /// were never claimed (ADR "debug from a selected execution" D5.5).
    ///
    /// A Running Loop row normally means a round was executing, and a restart fails it (while
    /// D6); an Iteration fails with the run's interrupted outer work. A reopened container that
    /// has not dispatched since has no work in flight: its positions are still to be dispatched,
    /// so boot must neither fail it as interrupted nor leave the run inert. It may be parked
    /// behind a terminal run (the reverse race) or awaiting dispatch in a running one.
    pub fn reopened_containers(
        &self,
        run: &str,
    ) -> Result<std::collections::HashSet<WorkflowNodeRunId>, DatabaseError> {
        self.pool.with_connection(|db| {
            Ok(db
                .prepare(REOPENED_CONTAINERS)?
                .query_map([run], |r| {
                    Ok(WorkflowNodeRunId::new(r.get::<_, String>(0)?))
                })?
                .collect::<Result<_, _>>()?)
        })
    }

    /// Includes the precise old attempts and container owners which must not settle during boot.
    pub fn protected_attempts(
        &self,
        run: &str,
    ) -> Result<std::collections::HashSet<WorkflowNodeRunId>, DatabaseError> {
        self.pool.with_connection(|db| {
            Ok(db
                .prepare("SELECT id FROM workflow_debug_boot_protected WHERE run_id=?1")?
                .query_map([run], |r| {
                    Ok(WorkflowNodeRunId::new(r.get::<_, String>(0)?))
                })?
                .collect::<Result<_, _>>()?)
        })
    }
}

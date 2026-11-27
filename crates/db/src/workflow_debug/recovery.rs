use super::*;
use ora_domain::WorkflowNodeRunId;

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

use super::*;
use rusqlite::{Row, Transaction};

const OPERATION: &str =
    "SELECT o.id,o.request_id,p.base_revision,o.state,o.revision,o.failure,p.plan,p.patch,
        o.unsettled_attempts,o.prior_status,o.prior_error,
        (SELECT node_run_id FROM workflow_debug_first_attempts bound WHERE bound.operation_id=o.id)
    FROM workflow_debug_operations o JOIN workflow_debug_previews p ON p.id=o.preview_id
    JOIN workflow_runs r ON r.id=o.run_id AND r.is_deleted=0";

/// Only the caller receiving Claimed may initiate stopping. Reconnects observe the durable operation.
pub enum DebugClaim {
    Claimed(WorkflowDebugHistoryEntry),
    Existing(WorkflowDebugHistoryEntry),
}

impl WorkflowDebugStore {
    /// Checks an existing idempotency key before the read set, including after a lost response.
    pub fn claim(&self, request: &CommitWorkflowDebugRequest) -> Result<DebugClaim, DatabaseError> {
        if request.request_id.trim().is_empty() {
            return Err(DebugRejection::EmptyRequestId.into());
        }
        self.pool.with_connection_mut(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let prior = tx
                .query_row(
                    &format!("{OPERATION} WHERE o.run_id=?1 AND o.request_id=?2"),
                    params![request.run_id, request.request_id],
                    history_row,
                )
                .optional()?;
            if let Some(entry) = prior {
                if entry.preview.preview_id != request.preview_id {
                    return Err(DebugRejection::RequestConflict.into());
                }
                return Ok(DebugClaim::Existing(entry));
            }
            // A new request meets the fence before its read set is revalidated: the page must
            // learn that the run is still settling an earlier confirmation, not that this
            // particular preview went stale.
            fence::ensure_inactive(&tx, &request.run_id)?;
            let revision = run_version(&tx, &request.run_id)?;
            let receipt = tx.query_row(
                "SELECT p.read_nodes,p.read_facts FROM workflow_debug_previews p
                 WHERE p.id=?1 AND p.run_id=?2 AND p.base_revision=?3
                   AND NOT EXISTS(SELECT 1 FROM workflow_debug_operations o WHERE o.preview_id=p.id)",
                params![request.preview_id,request.run_id,revision],
                |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))
            ).optional()?.ok_or(DebugRejection::StalePreview)?;
            if read_set::facts(&tx,&request.run_id,&receipt.0)? != serde_json::from_str::<Value>(&receipt.1)? {
                return Err(DebugRejection::StalePreview.into());
            }
            // Acceptance also keeps the run as it is now: a terminal run returns to that state
            // and reason when the debugged scope does not settle successfully.
            tx.execute(
                "INSERT INTO workflow_debug_operations(run_id,request_id,preview_id,state,
                        prior_status,prior_error,prior_output,prior_finished_at)
                SELECT r.id,?2,?3,'stopping',r.run_status,r.error,r.output,r.finished_at
                FROM workflow_runs r WHERE r.id=?1",
                params![request.run_id, request.request_id, request.preview_id],
            )?;
            let entry = tx.query_row(
                &format!("{OPERATION} WHERE o.id=?1"),
                [tx.last_insert_rowid()],
                history_row,
            )?;
            tx.commit()?;
            Ok(DebugClaim::Claimed(entry))
        })
    }

    /// Persists the coordinator's verified stop result and projection changes in a single commit.
    /// Called under the run gate after quiescence. A replay never re-enters the effect closure.
    ///
    /// A run that was terminal when the confirmation was accepted returns to `running` in the same
    /// commit; `now` stamps that change.
    pub fn apply_after_stop(
        &self,
        run: &str,
        request: &str,
        now: i64,
        settle: impl FnOnce(&Transaction<'_>, &WorkflowDebugPreview) -> Result<(), DatabaseError>,
    ) -> Result<WorkflowDebugOperation, DatabaseError> {
        self.pool.with_connection_mut(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let entry = tx.query_row(&format!("{OPERATION} WHERE o.run_id=?1 AND o.request_id=?2"),
                params![run,request],history_row).optional()?.ok_or(DebugRejection::InvalidTarget)?;
            if entry.operation.phase != WorkflowDebugPhase::Stopping {return Ok(entry.operation);}
            let revision = run_version(&tx,run)?;
            if revision != entry.operation.base_revision {return Err(DebugRejection::StalePreview.into());}
            let target_current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_debug_previews p
                JOIN workflow_runs r ON r.id=p.run_id JOIN workflow_run_root_scopes root ON root.run_id=r.id
                WHERE p.id=?1 AND p.snapshot_id=r.snapshot_id AND p.root_scope=root.scope_id
                AND EXISTS(SELECT 1 FROM workflow_execution_scopes s WHERE s.id=?3 AND s.run_id=r.id
                    AND (s.id=root.scope_id OR s.parent_loop_node_run_id=?6))
                AND (?6 IS NULL OR EXISTS(SELECT 1 FROM workflow_node_runs owner WHERE owner.id=?6 AND owner.run_id=r.id AND owner.scope_id=root.scope_id AND owner.is_deleted=0))
                AND CASE WHEN ?2 IS NOT NULL THEN EXISTS(SELECT 1 FROM workflow_node_runs n WHERE n.id=?2 AND n.run_id=r.id AND n.is_deleted=0)
                ELSE NOT EXISTS(SELECT 1 FROM workflow_node_runs n WHERE n.run_id=r.id AND n.scope_id=?3 AND n.node_id=?4 AND n.iteration IS ?5 AND n.is_deleted=0) END)",
                params![entry.preview.preview_id,entry.preview.target.node_run_id,entry.preview.target.scope_id,entry.preview.target.node_id,entry.preview.target.index,entry.preview.target.container_node_run_id],|r|r.get(0))?;
            if !target_current { return Err(DebugRejection::StalePreview.into()); }
            let revision = revision.checked_add(1).ok_or(DebugRejection::StalePreview)?;
            // A rejected projection must discard every tentative effect, including the
            // revision and terminal reactivation, before persisting the failure receipt.
            tx.execute_batch("SAVEPOINT debug_projection")?;
            // The strategy may create new attempts inside settle. They must see the new revision.
            tx.execute("UPDATE workflow_debug_operations SET state='applied',revision=?3
                WHERE run_id=?1 AND request_id=?2",params![run,request,revision])?;
            // A run accepted in a terminal state returns to running together with the revision,
            // but only while it still shows that very state. One that moved on while the stop was
            // in flight keeps its own outcome; recovery then reads the new revision.
            tx.execute(
                "UPDATE workflow_runs SET run_status=?2,error=NULL,output=NULL,finished_at=NULL,updated_at=?3
                WHERE id=?1 AND run_status=(SELECT prior_status FROM workflow_debug_operations
                    WHERE run_id=?1 AND request_id=?4) AND run_status<>?2",
                params![
                    run,
                    ora_domain::WorkflowRunStatus::Running.database_value(),
                    now,
                    request
                ],
            )?;
            let live_before: String = tx.query_row(
                "SELECT json_group_array(id) FROM workflow_node_runs WHERE run_id=?1 AND is_deleted=0",
                [run], |row| row.get(0),
            )?;
            match settle(&tx, &entry.preview) {
                Err(error @ DatabaseError::ForeachProjection(_)) => {
                    tx.execute_batch("ROLLBACK TO debug_projection; RELEASE debug_projection")?;
                    let reason = error.to_string();
                    tx.execute("UPDATE workflow_debug_operations SET state='stop_failed',failure=?3,unsettled_attempts='[]' WHERE run_id=?1 AND request_id=?2", params![run, request, reason])?;
                    tx.commit()?;
                    return Ok(WorkflowDebugOperation {
                        phase: WorkflowDebugPhase::StopFailed { reason, unsettled_node_run_ids: Vec::new() },
                        ..entry.operation
                    });
                }
                result => result?,
            }
            tx.execute_batch("RELEASE debug_projection")?;
            // Keep the first applied operation as the baseline until the run ends. A later
            // confirmation records its own acceptance state, but extends this dispatch scope.
            tx.execute(
                "INSERT INTO workflow_debug_dispatch(run_id,operation_id)
                 SELECT id,?2 FROM workflow_runs WHERE id=?1 AND run_status=?3
                 ON CONFLICT(run_id) DO NOTHING",
                params![run, entry.operation.operation_id, ora_domain::WorkflowRunStatus::Running.database_value()],
            )?;
            // An invalidated fact can represent zero or several physical attempts. Record the
            // rows the strategy actually retired, including successful siblings on an old path.
            tx.execute(
                "INSERT INTO workflow_debug_retired_attempts(node_run_id, operation_id)
                 SELECT n.id, ?2 FROM json_each(?1) live JOIN workflow_node_runs n ON n.id=live.value
                 WHERE n.is_deleted=1",
                params![live_before, entry.operation.operation_id],
            )?;
            tx.commit()?;
            Ok(WorkflowDebugOperation { phase:WorkflowDebugPhase::Applied {revision}, ..entry.operation })
        })
    }

    /// Records failed quiescence without creating a revision. A fresh request stays fenced.
    ///
    /// `unsettled_attempts` are the attempts the coordinator could not prove stopped or isolated.
    /// The receipt keeps them so the page can name what may still be running under the old
    /// revision; the same request key reads the receipt again after a reconnect.
    pub fn record_stop_failure(
        &self,
        run: &str,
        request: &str,
        reason: &str,
        unsettled_attempts: &[ora_domain::WorkflowNodeRunId],
    ) -> Result<bool, DatabaseError> {
        let unsettled: Vec<&str> = unsettled_attempts.iter().map(AsRef::as_ref).collect();
        self.pool.with_connection(|db| {
            Ok(db.execute(
                "UPDATE workflow_debug_operations SET state='stop_failed',failure=?3,unsettled_attempts=?4
            WHERE run_id=?1 AND request_id=?2 AND state='stopping'
            AND NOT EXISTS(SELECT 1 FROM json_each(?4) ids WHERE NOT EXISTS(
                SELECT 1 FROM workflow_node_runs n WHERE n.id=ids.value AND n.run_id=?1))",
                params![run, request, reason, serde_json::to_string(&unsettled)?],
            )? == 1)
        })
    }

    /// Releases a failed fence once the coordinator has independently proved isolation.
    pub fn release_failed_stop(&self, run: &str, request: &str) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|db| {
            Ok(db.execute(
                "UPDATE workflow_debug_operations SET released=1
            WHERE run_id=?1 AND request_id=?2 AND state='stop_failed' AND released=0",
                params![run, request],
            )? == 1)
        })
    }

    /// Reads the active dispatch fence: the newest stopping or unreleased failed operation's
    /// affected attempts and schedule scope. The scheduler holds these positions until the operation
    /// applies (or the failed fence is released); independent branches are not listed.
    pub fn dispatch_fence(
        &self,
        run: &str,
    ) -> Result<Option<ora_application::DebugDispatchFence>, DatabaseError> {
        self.pool.with_connection(|db| {
            let plan: Option<String> = db
                .query_row(
                    "SELECT p.plan FROM workflow_debug_operations o
                     JOIN workflow_debug_previews p ON p.id=o.preview_id
                     WHERE o.run_id=?1 AND (o.state='stopping' OR (o.state='stop_failed' AND o.released=0))
                     ORDER BY o.id DESC LIMIT 1",
                    [run],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(plan) = plan else { return Ok(None) };
            let plan: Value = serde_json::from_str(&plan)?;
            Ok(Some(ora_application::DebugDispatchFence {
                blocked_nodes: serde_json::from_value(plan["fenceNodes"].clone())?,
                affected_attempts: db.prepare("SELECT id FROM workflow_debug_boot_protected WHERE run_id=?1")?.query_map([run], |r|Ok(ora_domain::WorkflowNodeRunId::new(r.get::<_,String>(0)?)))?.collect::<Result<_,_>>()?,
            }))
        })
    }

    /// Lists accepted operations, their original preview and patch in durable acceptance order.
    pub fn history(&self, run: &str) -> Result<Vec<WorkflowDebugHistoryEntry>, DatabaseError> {
        self.pool.with_connection(|db| {
            let mut stmt = db.prepare(&format!("{OPERATION} WHERE o.run_id=?1 ORDER BY o.id"))?;
            Ok(stmt
                .query_map([run], history_row)?
                .collect::<Result<Vec<_>, _>>()?)
        })
    }
}

/// Decodes typed JSON from a text column, naming the column when the stored value is corrupt.
fn decoded<T: serde::de::DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let raw: String = row.get(index)?;
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

/// Checked inside the retry write transaction: a fence accepted after the scheduler's
/// optimistic read must still prevent replacing or waking this exact physical attempt.
pub(crate) fn attempt_fenced(db: &Connection, attempt: &str) -> rusqlite::Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM workflow_node_runs n JOIN workflow_debug_operations o ON o.run_id=n.run_id JOIN workflow_debug_previews p ON p.id=o.preview_id JOIN json_each(p.plan,'$.stop') stop ON stop.value=n.id WHERE n.id=?1 AND (o.state='stopping' OR (o.state='stop_failed' AND o.released=0)))", [attempt], |row|row.get(0))
}

/// Maps the persisted operation to the same receipt used for confirmation and history.
fn history_row(row: &Row<'_>) -> rusqlite::Result<WorkflowDebugHistoryEntry> {
    let phase = match row.get::<_, String>(3)?.as_str() {
        "applied" => WorkflowDebugPhase::Applied {
            revision: row.get(4)?,
        },
        "stopping" => WorkflowDebugPhase::Stopping,
        "stop_failed" => WorkflowDebugPhase::StopFailed {
            reason: row.get(5)?,
            unsettled_node_run_ids: decoded(row, 8)?,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(WorkflowDebugHistoryEntry {
        first_node_run_id: row.get(11)?,
        prior_run_status: run_status(row, 9)?,
        prior_error: row.get(10)?,
        operation: WorkflowDebugOperation {
            operation_id: row.get::<_, i64>(0)?.to_string(),
            request_id: row.get(1)?,
            base_revision: row.get(2)?,
            phase,
        },
        preview: decoded(row, 6)?,
        patch: decoded(row, 7)?,
    })
}

/// Reads the run status stored at acceptance in its wire form.
fn run_status(row: &Row<'_>, index: usize) -> rusqlite::Result<WorkflowRunStatus> {
    use ora_domain::WorkflowRunStatus as Stored;
    let stored = Stored::from_database_value(row.get(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(match stored {
        Stored::Pending => WorkflowRunStatus::Pending,
        Stored::Running => WorkflowRunStatus::Running,
        Stored::Succeeded => WorkflowRunStatus::Succeeded,
        Stored::Failed => WorkflowRunStatus::Failed,
        Stored::Cancelled => WorkflowRunStatus::Cancelled,
    })
}

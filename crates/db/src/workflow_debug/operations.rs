use super::*;
use rusqlite::{Row, Transaction};

const OPERATION: &str =
    "SELECT o.id,o.request_id,p.base_revision,o.state,o.revision,o.failure,p.plan,p.patch
    FROM workflow_debug_operations o JOIN workflow_debug_previews p ON p.id=o.preview_id
    JOIN workflow_runs r ON r.id=o.run_id AND r.is_deleted=0";

/// Only the caller receiving Claimed may initiate stopping. Reconnects observe the durable operation.
pub enum DebugClaim {
    Claimed(WorkflowDebugHistoryEntry),
    Existing(WorkflowDebugHistoryEntry),
}

impl WorkflowDebugStore {
    /// Checks an existing idempotency key before the epoch, including after a lost response.
    pub fn claim(&self, request: &CommitWorkflowDebugRequest) -> Result<DebugClaim, DatabaseError> {
        if request.request_id.trim().is_empty() {
            return Err(DebugRejection::RequestConflict.into());
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
            let (revision, epoch) = run_version(&tx, &request.run_id)?;
            let fresh: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM workflow_debug_previews p
                WHERE p.id=?1 AND p.run_id=?2 AND p.base_revision=?3 AND p.epoch=?4
                AND NOT EXISTS(SELECT 1 FROM workflow_debug_operations o WHERE o.preview_id=p.id))",
                params![request.preview_id, request.run_id, revision, epoch],
                |r| r.get(0),
            )?;
            if !fresh {
                return Err(DebugRejection::StalePreview.into());
            }
            if blocked(&tx, &request.run_id)? {
                return Err(DebugRejection::OperationInProgress.into());
            }
            tx.execute(
                "INSERT INTO workflow_debug_operations(run_id,request_id,preview_id,state)
                VALUES (?1,?2,?3,'stopping')",
                params![request.run_id, request.request_id, request.preview_id],
            )?;
            let entry = tx.query_row(
                &format!("{OPERATION} WHERE o.id=?1"),
                [tx.last_insert_rowid()],
                history_row,
            )?;
            // Even a failed stop may have external consequences. Never reuse sibling previews
            // from before the fence once it has been released.
            tx.execute(
                "UPDATE workflow_runs SET debug_epoch=debug_epoch+1 WHERE id=?1",
                [&request.run_id],
            )?;
            tx.commit()?;
            Ok(DebugClaim::Claimed(entry))
        })
    }

    /// Persists the coordinator's verified stop result and projection changes in a single commit.
    /// Called under the run gate after quiescence. A replay never re-enters the effect closure.
    pub fn apply_after_stop(
        &self,
        run: &str,
        request: &str,
        settle: impl FnOnce(&Transaction<'_>, &WorkflowDebugPreview) -> Result<(), DatabaseError>,
    ) -> Result<WorkflowDebugOperation, DatabaseError> {
        self.pool.with_connection_mut(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let entry = tx.query_row(&format!("{OPERATION} WHERE o.run_id=?1 AND o.request_id=?2"),
                params![run,request],history_row).optional()?.ok_or(DebugRejection::InvalidTarget)?;
            if entry.operation.phase != WorkflowDebugPhase::Stopping {return Ok(entry.operation);}
            let (revision, _) = run_version(&tx,run)?;
            if revision != entry.operation.base_revision {return Err(DebugRejection::StalePreview.into());}
            let target_current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_node_runs n
                JOIN workflow_debug_attempts a ON a.node_run_id=n.id JOIN workflow_runs r ON r.id=n.run_id
                JOIN workflow_execution_scopes s ON s.id=n.scope_id LEFT JOIN workflow_node_runs owner ON owner.id=s.parent_loop_node_run_id
                JOIN workflow_run_root_scopes root ON root.run_id=r.id
                WHERE r.id=?1 AND n.id=?2 AND n.is_deleted=0 AND r.snapshot_id=a.snapshot_id
                AND root.scope_id=COALESCE(owner.scope_id,s.id))",
                params![run,entry.preview.target.node_run_id],|r|r.get(0))?;
            if !target_current { return Err(DebugRejection::StalePreview.into()); }
            let revision = revision.checked_add(1).ok_or(DebugRejection::StalePreview)?;
            // The strategy may create new attempts inside settle. They must see the new revision.
            tx.execute("UPDATE workflow_debug_operations SET state='applied',revision=?3
                WHERE run_id=?1 AND request_id=?2",params![run,request,revision])?;
            settle(&tx,&entry.preview)?;
            tx.commit()?;
            Ok(WorkflowDebugOperation { phase:WorkflowDebugPhase::Applied {revision}, ..entry.operation })
        })
    }

    /// Records failed quiescence without creating a revision. A fresh request stays fenced.
    pub fn record_stop_failure(
        &self,
        run: &str,
        request: &str,
        reason: &str,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|db| {
            Ok(db.execute(
                "UPDATE workflow_debug_operations SET state='stop_failed',failure=?3
            WHERE run_id=?1 AND request_id=?2 AND state='stopping'",
                params![run, request, reason],
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

/// Reads the partial unique index's live-fence condition.
pub(super) fn blocked(db: &Connection, run: &str) -> Result<bool, rusqlite::Error> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_debug_operations WHERE run_id=?1
        AND (state='stopping' OR (state='stop_failed' AND released=0)))",
        [run],
        |r| r.get(0),
    )
}

/// Maps the persisted operation to the same receipt used for confirmation and history.
fn history_row(row: &Row<'_>) -> rusqlite::Result<WorkflowDebugHistoryEntry> {
    let json = |idx| -> rusqlite::Result<Value> {
        let raw: String = row.get(idx)?;
        serde_json::from_str(&raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    let phase = match row.get::<_, String>(3)?.as_str() {
        "applied" => WorkflowDebugPhase::Applied {
            revision: row.get(4)?,
        },
        "stopping" => WorkflowDebugPhase::Stopping,
        "stop_failed" => WorkflowDebugPhase::StopFailed {
            reason: row.get(5)?,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let decode = |err| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(err))
    };
    Ok(WorkflowDebugHistoryEntry {
        operation: WorkflowDebugOperation {
            operation_id: row.get::<_, i64>(0)?.to_string(),
            request_id: row.get(1)?,
            base_revision: row.get(2)?,
            phase,
        },
        preview: serde_json::from_value(json(6)?).map_err(decode)?,
        patch: serde_json::from_value(json(7)?).map_err(decode)?,
    })
}

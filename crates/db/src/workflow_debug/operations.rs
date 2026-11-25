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
    /// Checks an existing idempotency key before the read set, including after a lost response.
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
            // The strategy may create new attempts inside settle. They must see the new revision.
            tx.execute("UPDATE workflow_debug_operations SET state='applied',revision=?3
                WHERE run_id=?1 AND request_id=?2",params![run,request,revision])?;
            let live_before: String = tx.query_row(
                "SELECT json_group_array(id) FROM workflow_node_runs WHERE run_id=?1 AND is_deleted=0",
                [run], |row| row.get(0),
            )?;
            settle(&tx, &entry.preview)?;
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

    /// Reads the active dispatch fence: the newest stopping or unreleased failed operation's
    /// stop-set and schedule scope. The scheduler holds these positions until the operation
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
            let strings = |key: &str| -> Vec<String> {
                plan.get(key)
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Ok(Some(ora_application::DebugDispatchFence {
                blocked_nodes: strings("schedule"),
                stopped_attempts: strings("stop")
                    .into_iter()
                    .map(ora_domain::WorkflowNodeRunId::new)
                    .collect(),
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

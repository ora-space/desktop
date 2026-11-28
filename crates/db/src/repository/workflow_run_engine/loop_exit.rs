//! Persists the break boundary before any background session is stopped.
use super::*;
use ora_application::LoopExitState;
use std::collections::BTreeMap;

/// Freezes outputs, records the exit node, and fences every still-active sibling atomically.
pub(super) fn request(
    transaction: &Transaction<'_>,
    scope_id: &WorkflowScopeId,
    node_run_id: &WorkflowNodeRunId,
    result: &Result<BTreeMap<String, serde_json::Value>, String>,
    now: i64,
) -> Result<(), crate::DatabaseError> {
    let state: String = transaction.query_row(
        "SELECT state FROM workflow_execution_scopes WHERE id = ?1",
        params![scope_id.as_ref()],
        |row| row.get(0),
    )?;
    let mut state: LoopRoundExecutionState = serde_json::from_str(&state)?;
    if state.exit.is_some() {
        return Ok(());
    }
    let changed = transaction.execute("UPDATE workflow_node_runs SET status = 2, finished_at = ?3, updated_at = ?3 WHERE id = ?1 AND scope_id = ?2 AND node_type = 'loopExit' AND status = 1 AND is_deleted = 0", params![node_run_id.as_ref(), scope_id.as_ref(), now])?;
    if changed != 1 {
        return Err(crate::DatabaseError::IncompleteWorkflowRunContext);
    }
    // Capture exactly the rows fenced by this exit in the same transaction.
    let mut cancel = transaction.prepare(
        "UPDATE workflow_node_runs SET status=4, finished_at=?2, updated_at=?2
         WHERE scope_id=?1 AND status IN (0,1) AND is_deleted=0 RETURNING id",
    )?;
    let cancelled_attempts = cancel
        .query_map(params![scope_id.as_ref(), now], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    state.exit = Some(LoopExitState {
        cancelled_attempts: Some(cancelled_attempts),
        cleanup_result: None,
        node_run_id: node_run_id.to_string(),
        requested_at: now,
        result: result.clone(),
    });
    transaction.execute("UPDATE workflow_execution_scopes SET state = ?2, updated_at = ?3 WHERE id = ?1 AND status = 1", params![scope_id.as_ref(), serde_json::to_string(&state)?, now])?;
    Ok(())
}

/// Accepts a receipt only while the same exit still owns this round. Its write and the
/// terminal transition share the caller's transaction, including when an export is missing.
pub(super) fn acknowledge(
    transaction: &Transaction<'_>,
    scope_id: &WorkflowScopeId,
    node_run_id: &str,
    cleanup: &Result<(), String>,
) -> Result<Option<LoopRoundAdvance>, crate::DatabaseError> {
    let state: String = transaction.query_row(
        "SELECT state FROM workflow_execution_scopes WHERE id = ?1",
        [scope_id.as_ref()],
        |row| row.get(0),
    )?;
    let mut state: LoopRoundExecutionState = serde_json::from_str(&state)?;
    let Some(exit) = state
        .exit
        .as_mut()
        .filter(|exit| exit.node_run_id == node_run_id)
    else {
        return Ok(None);
    };
    exit.cleanup_result = Some(cleanup.clone());
    let advance = match cleanup.clone().and_then(|()| exit.result.clone()) {
        Ok(outputs) => LoopRoundAdvance::Succeed {
            outputs,
            reason: ora_application::LoopCompletionReason::LoopExit,
        },
        Err(error) => LoopRoundAdvance::Fail { error },
    };
    transaction.execute(
        "UPDATE workflow_execution_scopes SET state = ?2 WHERE id = ?1",
        params![scope_id.as_ref(), serde_json::to_string(&state)?],
    )?;
    Ok(Some(advance))
}

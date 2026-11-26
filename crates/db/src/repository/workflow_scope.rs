//! Transactional scope identity owned by the workflow execution repository.

use ora_domain::{
    WorkflowExecutionScope, WorkflowNodeRunId, WorkflowRunId, WorkflowScopeId, WorkflowScopeStatus,
};
use rusqlite::{Row, Transaction, params};

/// Reconstructs one persisted Loop round from its typed storage columns.
pub(super) fn map_round(row: &Row<'_>) -> Result<WorkflowExecutionScope, crate::DatabaseError> {
    let round_index = row.get::<_, i64>("round_index")?;
    Ok(WorkflowExecutionScope {
        id: WorkflowScopeId::new(row.get::<_, String>("id")?),
        run_id: WorkflowRunId::new(row.get::<_, String>("run_id")?),
        parent_loop_node_run_id: WorkflowNodeRunId::new(
            row.get::<_, String>("parent_loop_node_run_id")?,
        ),
        round_index: u32::try_from(round_index)
            .map_err(|_| crate::DatabaseError::CorruptWorkflowScopeRound(round_index))?,
        status: WorkflowScopeStatus::from_database_value(row.get("status")?)?,
        state: row.get("state")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// Lists every Loop round of one run in parent and round order for history projection.
pub(crate) fn list_rounds(
    connection: &rusqlite::Connection,
    run_id: &WorkflowRunId,
) -> Result<Vec<WorkflowExecutionScope>, crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at
         FROM workflow_execution_scopes
         WHERE run_id = ?1 AND parent_loop_node_run_id IS NOT NULL
         ORDER BY parent_loop_node_run_id, round_index",
    )?;
    let mut rows = statement.query(params![run_id.as_ref()])?;
    let mut scopes = Vec::new();
    while let Some(row) = rows.next()? {
        scopes.push(map_round(row)?);
    }
    Ok(scopes)
}

/// Loads the active round guarded by the migration's single-active-round constraint.
pub(super) fn find_active_round(
    connection: &rusqlite::Connection,
    parent_loop_node_run_id: &WorkflowNodeRunId,
) -> Result<Option<WorkflowExecutionScope>, crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, run_id, parent_loop_node_run_id, round_index, status, state, created_at, updated_at
         FROM workflow_execution_scopes
         WHERE parent_loop_node_run_id = ?1 AND status IN (0, 1)",
    )?;
    let mut rows = statement.query(params![parent_loop_node_run_id.as_ref()])?;
    rows.next()?.map(map_round).transpose()
}

/// Resolves the authoritative root so prepared dispatches retain their execution generation.
pub(super) fn current_root_scope(
    connection: &rusqlite::Connection,
    run_id: &WorkflowRunId,
) -> Result<WorkflowScopeId, crate::DatabaseError> {
    Ok(connection
        .query_row(
            "SELECT scope_id FROM workflow_run_root_scopes WHERE run_id = ?1",
            params![run_id.as_ref()],
            |row| row.get::<_, String>(0),
        )
        .map(WorkflowScopeId::new)?)
}

/// A restart gets a fresh root identity while historical node bindings remain immutable.
pub(super) fn restart_root_scope(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    now: i64,
) -> Result<(), crate::DatabaseError> {
    transaction.execute(
        "UPDATE workflow_execution_scopes SET status = 4, updated_at = ?2
         WHERE run_id = ?1 AND parent_loop_node_run_id IS NOT NULL AND status IN (0, 1)",
        params![run_id.as_ref(), now],
    )?;
    let scope_id: String = transaction.query_row(
        "INSERT INTO workflow_execution_scopes (id, run_id, created_at, updated_at)
         VALUES ('root:' || lower(hex(randomblob(16))), ?1, ?2, ?2) RETURNING id",
        params![run_id.as_ref(), now],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE workflow_run_root_scopes SET scope_id = ?2 WHERE run_id = ?1",
        params![run_id.as_ref(), scope_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;

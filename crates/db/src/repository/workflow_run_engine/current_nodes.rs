use ora_domain::WorkflowRunId;
use rusqlite::{OptionalExtension, Transaction, params};

/// Reads the `current_nodes` anchor from a run state JSON, treating a null state as empty.
pub(super) fn current_nodes_from_state(
    state: Option<&str>,
) -> Result<Vec<String>, crate::DatabaseError> {
    let Some(state) = state else {
        return Ok(Vec::new());
    };
    let value: serde_json::Value = serde_json::from_str(state)?;
    Ok(value
        .get("current_nodes")
        .and_then(serde_json::Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// Serializes a `current_nodes` anchor into the run state JSON.
pub(super) fn current_nodes_to_state(
    current_nodes: &[String],
) -> Result<String, crate::DatabaseError> {
    serde_json::to_string(&serde_json::json!({ "current_nodes": current_nodes }))
        .map_err(Into::into)
}

/// Rewrites the run's `current_nodes` anchor inside the active transaction.
pub(super) fn rewrite_current_nodes(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    now: i64,
    mutate: impl FnOnce(&mut Vec<String>),
) -> Result<(), crate::DatabaseError> {
    let state = transaction
        .query_row(
            "SELECT state FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
            params![run_id.as_ref()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let mut current_nodes = current_nodes_from_state(state.as_deref())?;
    mutate(&mut current_nodes);
    transaction.execute(
        "UPDATE workflow_runs SET state = ?2, updated_at = ?3 WHERE id = ?1 AND is_deleted = 0",
        params![
            run_id.as_ref(),
            current_nodes_to_state(&current_nodes)?,
            now
        ],
    )?;
    Ok(())
}

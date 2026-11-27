use super::*;

/// Reads a snapshot and overlays only edits visible at this position.
fn bound_graph(
    db: &Connection,
    run: &str,
    target: &WorkflowDebugTarget,
    snapshot: &str,
    revision: u32,
) -> Result<Value, DatabaseError> {
    let raw: String = db.query_row(
        "SELECT graph FROM workflow_snapshots WHERE id=?1",
        [snapshot],
        |row| row.get(0),
    )?;
    let mut graph: Value = serde_json::from_str(&raw)?;
    configuration::replay(db, run, target, snapshot, revision, &mut graph)?;
    Ok(graph)
}

/// Decodes the same physical identity for preview, configuration replay and attempt history.
pub(super) fn read_target(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowDebugTarget> {
    Ok(WorkflowDebugTarget {
        node_run_id: row.get("id")?,
        root_scope_id: None,
        scope_id: row.get("scope_id")?,
        node_id: row.get("node_id")?,
        container_node_run_id: row.get("container_attempt")?,
        index: row.get("iteration")?,
        round: row.get("round_index")?,
    })
}

impl WorkflowDebugStore {
    /// Retrieves a specific attempt even after invalidation. Legacy configuration remains unknown.
    pub fn attempt(
        &self,
        request: &GetWorkflowDebugAttemptRequest,
    ) -> Result<GetWorkflowDebugAttemptResponse, DatabaseError> {
        const ATTEMPT: &str = "SELECT n.id,n.scope_id,n.node_id,e.container_attempt,s.round_index,n.iteration,
                e.revision,e.snapshot_id,n.status,n.is_deleted,n.output,n.error,
                CASE WHEN json_valid(n.payload) THEN json_extract(n.payload,'$.debugCondition.branch') END AS branch,
                CASE WHEN n.node_type='agent' THEN e.dispatch_input IS NOT NULL
                     WHEN n.node_type='condition' THEN CASE WHEN json_valid(n.payload) THEN json_type(n.payload,'$.debugCondition') IS NOT NULL ELSE 0 END
                     ELSE e.entry_state IS NOT NULL END AS has_input,
                CAST(retired.operation_id AS TEXT) AS retired_by
            FROM workflow_node_runs n JOIN workflow_runs r ON r.id=n.run_id AND r.is_deleted=0
            JOIN workflow_execution_scopes s ON s.id=n.scope_id
            LEFT JOIN workflow_debug_attempts e ON e.node_run_id=n.id
            LEFT JOIN workflow_debug_retired_attempts retired ON retired.node_run_id=n.id
            WHERE n.run_id=?1 AND n.id=?2";
        self.pool.with_connection_mut(|connection| {
            let tx = connection.transaction()?;
            let mut attempt = tx
                .query_row(
                    ATTEMPT,
                    params![request.run_id, request.node_run_id],
                    |row| {
                        let status = match row.get::<_, i64>("status")? {
                            0 => WorkflowNodeStatus::Pending,
                            1 => WorkflowNodeStatus::Running,
                            2 => WorkflowNodeStatus::Succeeded,
                            3 => WorkflowNodeStatus::Failed,
                            4 => WorkflowNodeStatus::Cancelled,
                            _ => return Err(rusqlite::Error::InvalidQuery),
                        };
                        Ok(GetWorkflowDebugAttemptResponse {
                            target: read_target(row)?,
                            snapshot_id: row.get("snapshot_id")?,
                            revision: row.get("revision")?,
                            configuration: None,
                            status,
                            superseded: row.get("is_deleted")?,
                            retired_by_operation_id: row.get("retired_by")?,
                            input_recorded: row.get("has_input")?,
                            selected_branch: row.get("branch")?,
                            output: row.get("output")?,
                            error: row.get("error")?,
                        })
                    },
                )
                .optional()?
                .ok_or(DebugRejection::InvalidTarget)?;
            if let (Some(snapshot), Some(revision)) = (&attempt.snapshot_id, attempt.revision) {
                let graph = bound_graph(&tx, &request.run_id, &attempt.target, snapshot, revision)?;
                attempt.configuration = graph["nodes"]
                    .as_array()
                    .and_then(|nodes| nodes.iter().find(|n| n["id"] == attempt.target.node_id))
                    .and_then(|node| node.get("data"))
                    .cloned();
            }
            Ok(attempt)
        })
    }

    /// Supplies a revised graph to the runtime only after a revision has been applied.
    pub fn revised_graph(
        &self,
        run: &str,
        node_run: &str,
    ) -> Result<Option<WorkflowGraph>, DatabaseError> {
        const BINDING: &str = "SELECT n.id,n.scope_id,n.node_id,e.container_attempt,s.round_index,n.iteration,e.snapshot_id,e.revision
            FROM workflow_debug_attempts e JOIN workflow_node_runs n ON n.id=e.node_run_id
            JOIN workflow_execution_scopes s ON s.id=n.scope_id
            WHERE n.run_id=?1 AND n.id=?2 AND e.revision>0 AND n.is_deleted=0";
        self.pool.with_connection_mut(|connection| {
            let tx = connection.transaction()?;
            let binding = tx
                .query_row(BINDING, params![run, node_run], |row| {
                    Ok((
                        read_target(row)?,
                        row.get::<_, String>("snapshot_id")?,
                        row.get::<_, u32>("revision")?,
                    ))
                })
                .optional()?;
            let Some((target, snapshot, revision)) = binding else {
                return Ok(None);
            };
            let graph = bound_graph(&tx, run, &target, &snapshot, revision)?;
            Ok(Some(
                WorkflowGraph::parse(&graph.to_string())
                    .map_err(|_| DebugRejection::InvalidTarget)?,
            ))
        })
    }
}

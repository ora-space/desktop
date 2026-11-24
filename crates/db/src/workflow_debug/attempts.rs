use super::*;

/// Reads a snapshot exactly once and overlays only edits visible at this position.
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
        |r| r.get(0),
    )?;
    let mut graph: Value = serde_json::from_str(&raw)?;
    configuration::replay(db, run, target, snapshot, revision, &mut graph)?;
    Ok(graph)
}

impl WorkflowDebugStore {
    /// Retrieves a specific attempt even after retry or invalidation; legacy configurations are unknown.
    pub fn attempt(
        &self,
        request: &GetWorkflowDebugAttemptRequest,
    ) -> Result<GetWorkflowDebugAttemptResponse, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let tx=connection.transaction()?;
            let mut attempt=tx.query_row(
                "SELECT n.scope_id,n.node_id,e.container_attempt,s.round_index,n.iteration,e.revision,e.snapshot_id,
                    n.status,n.is_deleted,CASE WHEN n.node_type='agent' THEN e.dispatch_input IS NOT NULL
                    WHEN n.node_type='condition' THEN CASE WHEN json_valid(n.payload) THEN json_type(n.payload,'$.debugCondition') IS NOT NULL ELSE 0 END
                    ELSE e.entry_state IS NOT NULL END,n.output,n.error,
                    CASE WHEN json_valid(n.payload) THEN json_extract(n.payload,'$.debugCondition.branch') END
                 FROM workflow_node_runs n JOIN workflow_runs r ON r.id=n.run_id AND r.is_deleted=0
                 JOIN workflow_execution_scopes s ON s.id=n.scope_id
                 LEFT JOIN workflow_debug_attempts e ON e.node_run_id=n.id
                 WHERE n.run_id=?1 AND n.id=?2",
                params![request.run_id,request.node_run_id],|r| {
                    let status=match r.get::<_,i64>(7)? {
                        0=>WorkflowNodeStatus::Pending, 1=>WorkflowNodeStatus::Running, 2=>WorkflowNodeStatus::Succeeded,
                        3=>WorkflowNodeStatus::Failed, 4=>WorkflowNodeStatus::Cancelled, _=>return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(GetWorkflowDebugAttemptResponse {target:WorkflowDebugTarget {
                        node_run_id:request.node_run_id.clone(),scope_id:r.get(0)?,node_id:r.get(1)?,
                        container_node_run_id:r.get(2)?,round:r.get(3)?,index:r.get(4)?},
                        revision:r.get(5)?,snapshot_id:r.get(6)?,configuration:None,status,
                        superseded:r.get(8)?,input_recorded:r.get(9)?,output:r.get(10)?,error:r.get(11)?,selected_branch:r.get(12)?})
                }).optional()?.ok_or(DebugRejection::InvalidTarget)?;
            if let (Some(snapshot),Some(rev))=(&attempt.snapshot_id,attempt.revision) {
                let graph=bound_graph(&tx,&request.run_id,&attempt.target,snapshot,rev)?;
                attempt.configuration=graph["nodes"].as_array().and_then(|nodes|nodes.iter().find(|n|n["id"]==attempt.target.node_id))
                    .and_then(|node|node.get("data")).cloned();
            }
            Ok(attempt)
        })
    }

    /// Provides the frozen node configuration to the execution strategy for this attempt.
    /// Revision zero and pre-upgrade attempts continue through the existing snapshot path.
    pub fn revised_graph(
        &self,
        run: &str,
        node_run: &str,
    ) -> Result<Option<WorkflowGraph>, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let tx=connection.transaction()?;
            let bound=tx.query_row(
                "SELECT n.scope_id,n.node_id,e.container_attempt,s.round_index,n.iteration,e.snapshot_id,e.revision
                 FROM workflow_debug_attempts e JOIN workflow_node_runs n ON n.id=e.node_run_id
                 JOIN workflow_execution_scopes s ON s.id=n.scope_id
                 WHERE n.run_id=?1 AND n.id=?2 AND e.revision>0 AND n.is_deleted=0",
                params![run,node_run],|r|Ok((WorkflowDebugTarget {node_run_id:node_run.into(),scope_id:r.get(0)?,
                    node_id:r.get(1)?,container_node_run_id:r.get(2)?,round:r.get(3)?,index:r.get(4)?},
                    r.get::<_,String>(5)?,r.get::<_,u32>(6)?))).optional()?;
            let Some((target,snapshot,revision))=bound else{return Ok(None);};
            let graph=bound_graph(&tx,run,&target,&snapshot,revision)?;
            Ok(Some(WorkflowGraph::parse(&graph.to_string()).map_err(|_|DebugRejection::InvalidTarget)?))
        })
    }
}

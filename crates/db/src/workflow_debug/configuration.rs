use super::*;
use ora_application::render_variable_template;

/// Joins the row to its immutable snapshot and rejects superseded attempts and prior root executions.
pub(super) fn prepare(
    db: &Connection,
    request: &PreviewWorkflowDebugRequest,
) -> Result<DebugPlanningInput, DatabaseError> {
    let (target, snapshot, entry, dispatch, kind, graph_json) = db.query_row(
        "SELECT n.node_id,n.scope_id,n.iteration,s.round_index,e.container_attempt,
                e.snapshot_id,e.entry_state,CASE WHEN n.node_type='condition' AND json_valid(n.payload)
                    THEN json_extract(n.payload,'$.debugCondition') ELSE e.dispatch_input END AS observed_input,n.node_type,snap.graph,n.id
         FROM workflow_node_runs n JOIN workflow_runs r ON r.id=n.run_id
         JOIN workflow_run_root_scopes root ON root.run_id=r.id
         JOIN workflow_execution_scopes s ON s.id=n.scope_id
         LEFT JOIN workflow_node_runs parent ON parent.id=s.parent_loop_node_run_id
         LEFT JOIN workflow_debug_attempts e ON e.node_run_id=n.id
         JOIN workflow_snapshots snap ON snap.id=r.snapshot_id
         WHERE n.run_id=?1 AND n.id=?2 AND n.is_deleted=0
           AND (n.scope_id=root.scope_id OR (parent.scope_id=root.scope_id AND parent.is_deleted=0))
           AND (e.snapshot_id IS NULL OR e.snapshot_id=r.snapshot_id)",
        params![request.run_id, request.node_run_id],
        |row| Ok((
            super::attempts::read_target(row)?,
            row.get::<_, Option<String>>("snapshot_id")?,
            row.get::<_, Option<String>>("entry_state")?,
            row.get::<_, Option<String>>("observed_input")?,
            row.get::<_, String>("node_type")?,
            row.get::<_, String>("graph")?,
        )),
    ).optional()?.ok_or(DebugRejection::InvalidTarget)?;
    let snapshot = snapshot.ok_or(DebugRejection::MissingInputEvidence)?;
    let entry: Value = serde_json::from_str(&entry.ok_or(DebugRejection::MissingInputEvidence)?)?;
    let dispatch: Option<Value> = dispatch.as_deref().map(serde_json::from_str).transpose()?;
    if (target.index.is_some() || target.round.is_some()) && target.container_node_run_id.is_none()
    {
        return Err(DebugRejection::MissingInputEvidence.into());
    }
    let pool = if matches!(kind.as_str(), "agent" | "condition") {
        dispatch
            .as_ref()
            .and_then(|d| d.get("variablePool"))
            .cloned()
    } else {
        let field = if target.round.is_some() {
            "scopeState"
        } else {
            "runPayload"
        };
        entry[field]
            .as_str()
            .map(serde_json::from_str::<Value>)
            .transpose()?
            .and_then(|state| state.get("variablePool").cloned())
    }
    .ok_or(DebugRejection::MissingInputEvidence)?;
    let mut pool: WorkflowVariablePool = serde_json::from_value(pool)?;
    let mut graph: Value = serde_json::from_str(&graph_json)?;
    replay(
        db,
        &request.run_id,
        &target,
        &snapshot,
        request.base_revision,
        &mut graph,
    )?;
    let data = graph["nodes"]
        .as_array_mut()
        .and_then(|nodes| {
            nodes
                .iter_mut()
                .find(|n| n["id"].as_str() == Some(target.node_id.as_str()))
        })
        .and_then(|node| node.get_mut("data"))
        .ok_or(DebugRejection::InvalidTarget)?;
    replace(data, &kind, &request.patch)?;
    let candidate =
        WorkflowGraph::parse(&graph.to_string()).map_err(|e| invalid("", &e.to_string()))?;
    // Foreach reuses root-pool selector names. A value left by a skipped producer in a
    // preceding item cannot satisfy a newly edited prompt or condition in this item.
    if let Some(index) = target.index {
        let completed = entry["completedAttempts"]
            .as_array()
            .ok_or(DebugRejection::MissingInputEvidence)?;
        let mut query = db.prepare(
            "SELECT n.node_id FROM json_each(?1) claimed
            JOIN workflow_node_runs n ON n.id=claimed.value WHERE n.scope_id=?2 AND n.iteration=?3",
        )?;
        let writers = query
            .query_map(
                params![serde_json::to_string(completed)?, target.scope_id, index],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        pool.values.retain(|key, _| {
            pool.catalog.get(key).is_some_and(|declaration| {
                candidate.region_owner(&declaration.writer).is_none()
                    || writers.contains(&declaration.writer)
            })
        });
    }
    let node = candidate
        .execution_node(&target.node_id)
        .ok_or(DebugRejection::InvalidTarget)?;
    if let Some(agent) = &node.agent_config {
        render_variable_template(&agent.prompt, &pool)
            .map_err(|e| invalid("/agentConfig/prompt", &e.to_string()))?;
    }
    let predicted_branch = node
        .condition_config
        .as_ref()
        .map(|condition| condition.preview_branch(&pool))
        .transpose()
        .map_err(|e| invalid("/cases", &e.to_string()))?;
    if let Some(iteration) = &node.iteration_config {
        let items = pool
            .resolve(&iteration.iterator_selector)
            .map_err(|e| invalid("/iterationConfig/iteratorSelector", &e.to_string()))?
            .and_then(Value::as_array)
            .ok_or_else(|| {
                invalid(
                    "/iterationConfig/iteratorSelector",
                    "expected assigned array",
                )
            })?;
        if items.len() > iteration.max_iterations as usize {
            return Err(invalid(
                "/iterationConfig/iteratorSelector",
                "array exceeds the container limit",
            )
            .into());
        }
    }
    let run = ora_domain::WorkflowRunId::new(request.run_id.clone());
    let current_attempts = crate::repository::list_node_runs(db, &run)?;
    let current_scopes = crate::repository::list_rounds(db, &run)?;
    let raw: Option<String> = db.query_row(
        "SELECT payload FROM workflow_runs WHERE id=?1",
        [&request.run_id],
        |r| r.get(0),
    )?;
    let current_payload = raw.as_deref().map(serde_json::from_str).transpose()?;
    Ok(DebugPlanningInput {
        target,
        candidate,
        pool,
        entry,
        dispatch,
        current_attempts,
        current_scopes,
        current_payload,
        predicted_branch,
    })
}

/// Replays patches in revision order, preserving each field's container and suffix boundary.
pub(super) fn replay(
    db: &Connection,
    run: &str,
    target: &WorkflowDebugTarget,
    snapshot: &str,
    revision: u32,
    data: &mut Value,
) -> Result<(), DatabaseError> {
    let mut stmt = db.prepare(
        "SELECT p.patch, n.node_type,n.node_id FROM workflow_debug_operations op
        JOIN workflow_debug_previews p ON p.id=op.preview_id
        JOIN workflow_node_runs n ON n.id=p.target
        JOIN workflow_debug_attempts e ON e.node_run_id=n.id
        JOIN workflow_execution_scopes s ON s.id=n.scope_id
        LEFT JOIN workflow_node_runs parent ON parent.id=s.parent_loop_node_run_id
        WHERE op.run_id=?1 AND op.revision<=?2 AND e.snapshot_id=?3
          AND (e.container_attempt IS NULL OR e.container_attempt=?4)
          AND (n.iteration IS NULL OR n.iteration<=?5)
          AND (s.round_index IS NULL OR s.round_index<=?6)
          AND COALESCE(parent.scope_id,n.scope_id)=(SELECT COALESCE(owner.scope_id,scope.id)
              FROM workflow_execution_scopes scope LEFT JOIN workflow_node_runs owner
              ON owner.id=scope.parent_loop_node_run_id WHERE scope.id=?7)
        ORDER BY op.revision",
    )?;
    let mut rows = stmt.query(params![
        run,
        revision,
        snapshot,
        target.container_node_run_id,
        target.index,
        target.round,
        target.scope_id
    ])?;
    while let Some(row) = rows.next()? {
        let patch: Vec<WorkflowDebugPatch> = serde_json::from_str(&row.get::<_, String>(0)?)?;
        let id = row.get::<_, String>(2)?;
        let node_data = data["nodes"]
            .as_array_mut()
            .and_then(|nodes| nodes.iter_mut().find(|n| n["id"] == id))
            .and_then(|n| n.get_mut("data"))
            .ok_or(DebugRejection::InvalidTarget)?;
        replace(node_data, &row.get::<_, String>(1)?, &patch)?;
    }
    Ok(())
}

/// Applies only leaf edits to existing configuration; branch IDs, topology and declarations stay immutable.
fn replace(
    data: &mut Value,
    kind: &str,
    patches: &[WorkflowDebugPatch],
) -> Result<(), DebugRejection> {
    if patches.is_empty() {
        return Err(invalid("", "empty patch"));
    }
    for WorkflowDebugPatch::Replace { path, value } in patches {
        let parts: Vec<_> = path.split('/').collect();
        let index = |part: &str| part.parse::<usize>().is_ok_and(|n| n.to_string() == part);
        let allowed = match (kind, parts.as_slice()) {
            (
                "agent",
                ["", "agentConfig", "prompt"] | ["", "agentConfig", "executor", "modelId"],
            ) => value.is_string(),
            ("condition", ["", "cases", i, "logic"]) => index(i),
            (
                "condition",
                [
                    "",
                    "cases",
                    i,
                    "conditions",
                    j,
                    "operator" | "value" | "variableSelector",
                ],
            ) => index(i) && index(j),
            ("loop", ["", "loopConfig", "until" | "maxIterations"]) => true,
            ("loop", ["", "loopConfig", "variables", i, "feedback"]) => index(i),
            (
                "iteration",
                [
                    "",
                    "iterationConfig",
                    "iteratorSelector" | "collectSelector" | "errorStrategy",
                ],
            ) => true,
            _ => false,
        };
        if !allowed {
            return Err(invalid(path, "field is not editable"));
        }
        *data
            .pointer_mut(path)
            .ok_or_else(|| invalid(path, "field does not exist"))? = value.clone();
    }
    Ok(())
}

/// Reports the precise data-relative path to the editing surface.
fn invalid(path: &str, reason: &str) -> DebugRejection {
    DebugRejection::InvalidPatch {
        path: path.into(),
        reason: reason.into(),
    }
}

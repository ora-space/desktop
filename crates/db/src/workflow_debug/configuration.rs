use super::*;
use ora_application::render_variable_template;

/// Joins the row to its immutable snapshot and rejects superseded attempts and prior root executions.
pub(super) fn prepare(
    db: &Connection,
    request: &PreviewWorkflowDebugRequest,
) -> Result<DebugPlanningInput, DatabaseError> {
    if let WorkflowDebugSelection::NodeRunId(id) = &request.selection {
        // An attempt recorded before the current snapshot baseline keeps its inputs as history
        // only; the page has to select a position the current baseline has produced.
        let recorded = db
            .query_row(
                "SELECT e.revision,COALESCE((SELECT MAX(b.revision) FROM workflow_debug_baselines b
                    WHERE b.run_id=n.run_id),0)
                 FROM workflow_node_runs n JOIN workflow_debug_attempts e ON e.node_run_id=n.id
                 WHERE n.run_id=?1 AND n.id=?2 AND n.is_deleted=0",
                params![request.run_id, id],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?;
        if let Some((revision, baseline)) = recorded
            && revision < baseline
        {
            return Err(DebugRejection::MissingFact(format!(
                "{id} was recorded under revision {revision}, before the current snapshot baseline {baseline}"
            ))
            .into());
        }
    }
    let (target, snapshot, entry, dispatch, kind, graph_json, evidence_source) = match &request.selection {
        WorkflowDebugSelection::PendingTarget(position) => super::pending::resolve(db, &request.run_id, position)?,
        WorkflowDebugSelection::NodeRunId(id) => db.query_row(
        "SELECT n.node_id,n.scope_id,n.iteration,s.round_index,e.container_attempt,
                e.snapshot_id,entry.entry_state,CASE WHEN n.node_type='condition' AND json_valid(n.payload)
                    THEN json_extract(n.payload,'$.debugCondition') ELSE COALESCE(entry.dispatch_input, prior_entry.dispatch_input) END AS observed_input,n.node_type,snap.graph,n.id,CASE WHEN e.dispatch_input IS NULL THEN prior.node_run_id END AS evidence_source
         FROM workflow_node_runs n JOIN workflow_runs r ON r.id=n.run_id
         JOIN workflow_run_root_scopes root ON root.run_id=r.id
         JOIN workflow_execution_scopes s ON s.id=n.scope_id
         LEFT JOIN workflow_node_runs parent ON parent.id=s.parent_loop_node_run_id
          LEFT JOIN workflow_debug_attempts e ON e.node_run_id=n.id
          LEFT JOIN workflow_debug_entries entry ON entry.node_run_id=n.id
          LEFT JOIN workflow_debug_attempts prior ON prior.node_run_id=(
            SELECT candidate.id FROM json_each(CASE WHEN json_valid(n.payload) THEN n.payload END,'$.retry_chain') chain
            JOIN workflow_node_runs candidate ON candidate.id=chain.value
            JOIN workflow_debug_attempts evidence ON evidence.node_run_id=candidate.id
            WHERE n.status=1 AND n.started_at IS NULL AND json_extract(CASE WHEN json_valid(n.payload) THEN n.payload END,'$.retry_wait.previous_node_run_id') IS NOT NULL
              AND evidence.dispatch_input IS NOT NULL ORDER BY chain.key DESC LIMIT 1)
            AND prior.snapshot_id=e.snapshot_id AND prior.revision=e.revision AND prior.container_attempt IS e.container_attempt
            AND EXISTS (SELECT 1 FROM workflow_node_runs candidate WHERE candidate.id=prior.node_run_id
                AND candidate.is_deleted=1 AND candidate.status=3 AND candidate.run_id=n.run_id
                AND candidate.scope_id=n.scope_id AND candidate.node_id=n.node_id AND candidate.iteration IS n.iteration)
         LEFT JOIN workflow_debug_entries prior_entry ON prior_entry.node_run_id=prior.node_run_id
         JOIN workflow_snapshots snap ON snap.id=r.snapshot_id
         WHERE n.run_id=?1 AND n.id=?2 AND n.is_deleted=0
           AND (n.scope_id=root.scope_id OR (parent.scope_id=root.scope_id AND parent.is_deleted=0))
           AND (e.snapshot_id IS NULL OR e.snapshot_id=r.snapshot_id)",
        params![request.run_id, id],
        |row| Ok((
            super::attempts::read_target(row)?,
            row.get::<_, Option<String>>("snapshot_id")?,
            row.get::<_, Option<String>>("entry_state")?,
            row.get::<_, Option<String>>("observed_input")?,
            row.get::<_, String>("node_type")?,
            row.get::<_, String>("graph")?,
            row.get::<_, Option<String>>("evidence_source")?,
        )),
    ).optional()?.ok_or(DebugRejection::InvalidTarget)?,
    };
    if !matches!(kind.as_str(), "agent" | "condition" | "loop" | "iteration") {
        return Err(DebugRejection::InvalidTarget.into());
    }
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
    let previous =
        WorkflowGraph::parse(&graph.to_string()).map_err(|e| invalid("", &e.to_string()))?;
    let data = graph["nodes"]
        .as_array_mut()
        .and_then(|nodes| {
            nodes
                .iter_mut()
                .find(|n| n["id"].as_str() == Some(target.node_id.as_str()))
        })
        .and_then(|node| node.get_mut("data"))
        .ok_or(DebugRejection::InvalidTarget)?;
    // The first run page only offers variables that are declared as arrays at the root. A path
    // into a nested array is reported on its own field instead of failing graph validation as a
    // whole.
    for WorkflowDebugPatch::Replace { path, value } in &request.patch {
        let root_variable = matches!(
            value.as_array().map(Vec::as_slice),
            Some([node, variable])
                if node.as_str().is_some_and(|node| !node.is_empty())
                    && variable.as_str().is_some_and(|variable| !variable.is_empty())
        );
        if path == "/iterationConfig/iteratorSelector" && !root_variable {
            return Err(invalid(
                path,
                "expected the selector of a variable declared as an array at the root",
            )
            .into());
        }
    }
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
    let member_configurations = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|member| {
            let id = member["id"].as_str()?;
            let owner = candidate
                .region_owner(id)
                .or_else(|| candidate.loop_owner(id));
            (owner == Some(target.node_id.as_str()))
                .then(|| (id.to_owned(), member["data"].clone()))
        })
        .collect();
    let mut read_nodes = super::read_set::nodes(&previous, &target.node_id);
    read_nodes.extend(super::read_set::nodes(&candidate, &target.node_id));
    Ok(DebugPlanningInput {
        evidence_source,
        read_nodes,
        member_configurations,
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
/// Revisions at or below the snapshot baseline belong to an earlier configuration lineage and
/// stay history, also when a restart moves the target into a new root.
pub(super) fn replay(
    db: &Connection,
    run: &str,
    target: &WorkflowDebugTarget,
    snapshot: &str,
    revision: u32,
    data: &mut Value,
) -> Result<(), DatabaseError> {
    for node in data["nodes"].as_array_mut().into_iter().flatten() {
        if let Some(node_data) = node.get_mut("data") {
            super::normalize::data(node_data);
        }
    }
    let mut stmt = db.prepare(
        "SELECT p.patch,p.node_type,json_extract(p.plan,'$.target.nodeId') FROM workflow_debug_operations op
        JOIN workflow_debug_previews p ON p.id=op.preview_id
        WHERE op.run_id=?1 AND op.revision<=?2 AND p.snapshot_id=?3
          AND op.revision>COALESCE((SELECT MAX(b.revision) FROM workflow_debug_baselines b
              WHERE b.run_id=?1 AND b.revision<=?2),0)
          AND (json_extract(p.plan,'$.target.containerNodeRunId') IS NULL OR json_extract(p.plan,'$.target.containerNodeRunId')=?4)
          AND COALESCE(json_extract(p.plan,'$.target.index'),-1)<=COALESCE(?5,-1)
          AND COALESCE(json_extract(p.plan,'$.target.round'),-1)<=COALESCE(?6,-1)
        ORDER BY op.revision",
    )?;
    let mut rows = stmt.query(params![
        run,
        revision,
        snapshot,
        target.container_node_run_id,
        target.index,
        target.round
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

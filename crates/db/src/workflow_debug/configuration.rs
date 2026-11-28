use super::*;
use WorkflowDebugReason as Reason;
use ora_application::{ConditionError, VariableTemplateError, render_variable_template};

#[cfg(test)]
mod form_samples;

/// Joins the row to its immutable snapshot and rejects superseded attempts and prior root executions.
/// Apply uses the source already verified by preview and claim: quiescence has cancelled the retry
/// wait by then. The snapshot, revision, container and coordinates must still match its predecessor.
pub(super) fn prepare(
    db: &Connection,
    request: &PreviewWorkflowDebugRequest,
    confirmed_retry_source: Option<&str>,
) -> Result<DebugPlanningInput, DatabaseError> {
    let super::context::SelectedContext {
        target,
        entry,
        dispatch,
        kind,
        mut pool,
        mut graph,
        evidence_source,
    } = super::context::load(
        db,
        &request.run_id,
        &request.selection,
        request.base_revision,
        confirmed_retry_source,
    )?;
    let previous = WorkflowGraph::parse(&graph.to_string())
        .map_err(|e| invalid("", DebugPatchReason::Validation(e.to_string())))?;
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
            return Err(invalid(path, Reason::RootArrayRequired).into());
        }
    }
    replace(data, &kind, &request.patch)?;
    let candidate = WorkflowGraph::parse(&graph.to_string())
        .map_err(|e| invalid("", DebugPatchReason::Validation(e.to_string())))?;
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
            .map_err(|e| invalid("/agentConfig/prompt", e))?;
    }
    let predicted_branch = node
        .condition_config
        .as_ref()
        .map(|condition| condition.preview_branch(&pool))
        .transpose()
        .map_err(|e| invalid("/cases", e))?;
    if let Some(iteration) = &node.iteration_config {
        let items = pool
            .resolve(&iteration.iterator_selector)
            .map_err(|_| {
                invalid(
                    "/iterationConfig/iteratorSelector",
                    Reason::InvalidVariablePath {
                        variable: iteration.iterator_selector.qualified(),
                    },
                )
            })?
            .and_then(Value::as_array)
            .ok_or_else(|| {
                invalid(
                    "/iterationConfig/iteratorSelector",
                    Reason::ExpectedArray {
                        variable: iteration.iterator_selector.qualified(),
                    },
                )
            })?;
        if items.len() > iteration.max_iterations as usize {
            return Err(invalid(
                "/iterationConfig/iteratorSelector",
                Reason::ArrayExceedsLimit {
                    variable: iteration.iterator_selector.qualified(),
                    limit: iteration.max_iterations,
                },
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
        return Err(invalid("", Reason::EmptyPatch));
    }
    for WorkflowDebugPatch::Replace { path, value } in patches {
        let parts: Vec<_> = path.split('/').collect();
        let index = |part: &str| part.parse::<usize>().is_ok_and(|n| n.to_string() == part);
        let allowed = match (kind, parts.as_slice()) {
            (
                "agent",
                ["", "agentConfig", "prompt"] | ["", "agentConfig", "executor", "modelId"],
            ) => true,
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
            return Err(invalid(path, Reason::FieldNotEditable));
        }
        // Validate field shapes before graph parsing loses the edited JSON pointer. Condition
        // comparison values deliberately accept arbitrary JSON, including null.
        let wrong_type = match parts.last().copied() {
            Some("prompt" | "modelId") if !value.is_string() => Some(Reason::ExpectedText),
            Some("logic" | "operator" | "errorStrategy")
                if !value.is_null() && !value.is_string() =>
            {
                Some(Reason::ExpectedText)
            }
            Some("variableSelector" | "feedback" | "collectSelector")
                if !value
                    .as_array()
                    .is_some_and(|parts| parts.iter().all(Value::is_string)) =>
            {
                Some(Reason::ExpectedSelector)
            }
            Some("maxIterations") if value.as_u64().is_none_or(|n| u32::try_from(n).is_err()) => {
                Some(Reason::ExpectedInteger)
            }
            Some("until") if !value.is_object() => Some(Reason::ExpectedObject),
            _ => None,
        };
        if let Some(reason) = wrong_type {
            return Err(invalid(path, reason));
        }
        *data
            .pointer_mut(path)
            .ok_or_else(|| invalid(path, Reason::FieldMissing))? = value.clone();
    }
    Ok(())
}

/// Reports the precise data-relative path to the editing surface.
fn invalid(path: &str, reason: impl Into<DebugPatchReason>) -> DebugRejection {
    DebugRejection::InvalidPatch {
        path: path.into(),
        reason: reason.into(),
    }
}

impl From<VariableTemplateError> for DebugPatchReason {
    /// Preserves the variable identity while adapting evaluator errors for the run editor.
    fn from(e: VariableTemplateError) -> Self {
        match e {
            VariableTemplateError::Undeclared { selector }
            | VariableTemplateError::UnsetVariable { selector } => {
                Reason::VariableUnavailable { variable: selector }.into()
            }
            VariableTemplateError::InvalidPath { selector, segment } => {
                Reason::InvalidVariablePath {
                    variable: format!("{selector}.{segment}"),
                }
                .into()
            }
            VariableTemplateError::InvalidSelector { selector }
            | VariableTemplateError::PathNotObject { selector } => {
                Reason::InvalidVariablePath { variable: selector }.into()
            }
            VariableTemplateError::Unterminated => DebugPatchReason::Validation(e.to_string()),
        }
    }
}

impl From<ConditionError> for DebugPatchReason {
    /// Preserves the variable identity while adapting evaluator errors for the run editor.
    fn from(e: ConditionError) -> Self {
        match e {
            ConditionError::Undeclared { selector }
            | ConditionError::UnsetVariable { selector } => {
                Reason::VariableUnavailable { variable: selector }.into()
            }
            ConditionError::InvalidPath { selector, segment } => Reason::InvalidVariablePath {
                variable: format!("{selector}.{segment}"),
            }
            .into(),
            ConditionError::InvalidSelector { selector } => Reason::InvalidVariablePath {
                variable: selector.join("."),
            }
            .into(),
            ConditionError::TypeMismatch {
                selector, operator, ..
            } => Reason::ComparisonTypeMismatch {
                variable: selector,
                operator: operator.to_string(),
            }
            .into(),
            other => DebugPatchReason::Validation(other.to_string()),
        }
    }
}

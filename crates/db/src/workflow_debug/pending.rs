//! Resolve a logical position against the same ready projection that dispatch uses.
use super::*;
use ora_application::{
    BranchProjection, LoopRoundExecutionState, ProjectedNodeState, WorkflowRunPayload,
};
use serde_json::json;

type Evidence = (
    WorkflowDebugTarget,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
);

pub(super) fn resolve(
    db: &Connection,
    run: &str,
    pos: &PendingWorkflowDebugTarget,
) -> Result<Evidence, DatabaseError> {
    let missing = |reason: &str| DebugRejection::MissingFact(reason.to_owned());
    let (snapshot,raw,payload)=db.query_row(
        "SELECT r.snapshot_id,s.graph,r.payload FROM workflow_runs r JOIN workflow_snapshots s ON s.id=r.snapshot_id
         JOIN workflow_run_root_scopes root ON root.run_id=r.id WHERE r.id=?1 AND root.scope_id=?2",
        params![run,pos.root_scope_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))
        .optional()?.ok_or(DebugRejection::InvalidTarget)?;
    // Container source and control settings may themselves have been edited in this run.
    let mut active: Value = serde_json::from_str(&raw)?;
    let position = WorkflowDebugTarget {
        node_run_id: None,
        root_scope_id: Some(pos.root_scope_id.clone()),
        scope_id: pos.scope_id.clone(),
        node_id: pos.node_id.clone(),
        container_node_run_id: pos.container_node_run_id.clone(),
        round: None,
        index: pos.index,
    };
    super::configuration::replay(
        db,
        run,
        &position,
        &snapshot,
        run_version(db, run)?,
        &mut active,
    )?;
    let graph =
        WorkflowGraph::parse(&active.to_string()).map_err(|_| DebugRejection::InvalidTarget)?;
    let kind = graph
        .execution_node(&pos.node_id)
        .ok_or(DebugRejection::InvalidTarget)?
        .node_type
        .as_str();
    let payload: WorkflowRunPayload = payload
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let rows =
        crate::repository::list_node_runs(db, &ora_domain::WorkflowRunId::new(run.to_owned()))?;
    let scoped: Vec<_> = rows
        .iter()
        .filter(|r| r.scope_id.as_ref() == pos.scope_id)
        .cloned()
        .collect();
    let mut owner_id = None;
    let mut round = None;
    let (mut pool, ready) = if let Some(owner) = graph.loop_owner(&pos.node_id) {
        let (id,number,state)=db.query_row(
            "SELECT n.id,s.round_index,s.state FROM workflow_execution_scopes s JOIN workflow_node_runs n ON n.id=s.parent_loop_node_run_id
             WHERE s.run_id=?1 AND s.id=?2 AND s.status=1 AND n.scope_id=?3 AND n.node_id=?4 AND n.is_deleted=0 AND n.status=1",
            params![run,pos.scope_id,pos.root_scope_id,owner],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?)))
            .optional()?.ok_or_else(||missing("scopeId must identify a live round of the selected Loop"))?;
        if pos.index.is_some() || pos.container_node_run_id.as_ref().is_some_and(|v| v != &id) {
            return Err(DebugRejection::InvalidTarget.into());
        }
        let state: LoopRoundExecutionState = serde_json::from_str(&state)?;
        if state.exit.is_some() {
            return Err(missing("round has already exited").into());
        }
        let (_, body) = graph
            .loop_body(owner)
            .ok_or(DebugRejection::InvalidTarget)?;
        let ready =
            BranchProjection::new(body, &scoped, &state.condition_decisions).state(&pos.node_id);
        owner_id = Some(id);
        round = Some(number);
        (state.variable_pool, ready)
    } else if let Some(owner) = graph.region_owner(&pos.node_id) {
        let id = pos
            .container_node_run_id
            .as_ref()
            .ok_or_else(|| missing("containerNodeRunId is required for a foreach member"))?;
        let index = pos
            .index
            .ok_or_else(|| missing("index is required for a foreach member"))?;
        if pos.scope_id != pos.root_scope_id
            || !scoped.iter().any(|r| {
                r.id.as_ref() == id
                    && r.node_id == owner
                    && r.status == ora_domain::WorkflowNodeStatus::Running
            })
        {
            return Err(DebugRejection::InvalidTarget.into());
        }
        let config = graph
            .node(owner)
            .and_then(|n| n.iteration_config.as_ref())
            .ok_or(DebugRejection::InvalidTarget)?;
        let frozen: Option<String> = db
            .query_row(
                "SELECT json_extract(entry_state,'$.runPayload') FROM workflow_debug_attempts
             WHERE node_run_id=?1 AND snapshot_id=?2",
                params![id, snapshot],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let frozen: WorkflowRunPayload = serde_json::from_str(
            &frozen.ok_or_else(|| missing("foreach container entrance has no frozen input"))?,
        )?;
        let source = frozen
            .variable_pool
            .resolve(&config.iterator_selector)
            .ok()
            .flatten()
            .and_then(Value::as_array);
        let item = source.and_then(|items| items.get(index as usize));
        if item.is_none()
            || payload
                .variable_pool
                .values
                .get(&format!("{owner}.index"))
                .and_then(Value::as_u64)
                != Some(u64::from(index))
            || payload.variable_pool.values.get(&format!("{owner}.item")) != item
            || payload
                .iteration_ledger(owner)
                .is_some_and(|l| l.contains_key(&index))
        {
            return Err(missing("index must identify the bound, unsettled foreach item").into());
        }
        owner_id = Some(id.clone());
        let decisions = payload.iteration_round_decisions(index);
        let ready = BranchProjection::new_region_round(
            &graph,
            owner,
            graph.region(owner).ok_or(DebugRejection::InvalidTarget)?,
            index,
            &scoped,
            &decisions,
        )
        .state(&pos.node_id);
        (payload.variable_pool.clone(), ready)
    } else {
        if pos.scope_id != pos.root_scope_id
            || pos.index.is_some()
            || pos.container_node_run_id.is_some()
        {
            return Err(DebugRejection::InvalidTarget.into());
        }
        let ready = BranchProjection::new(&graph, &scoped, &payload.resolved_condition_decisions())
            .state(&pos.node_id);
        (payload.variable_pool.clone(), ready)
    };
    if ready != ProjectedNodeState::Ready {
        return Err(missing(&format!(
            "{} in {} is not ready for first dispatch; prerequisites: {}",
            pos.node_id,
            pos.scope_id,
            graph
                .execution_scopes()
                .into_iter()
                .find(|g| g.node(&pos.node_id).is_some())
                .map(|g| g
                    .predecessors(&pos.node_id)
                    .into_iter()
                    .map(|n| &n.id)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        ))
        .into());
    }
    // Only committed producers and the already-bound container entrance can supply inputs.
    // Imported while values were frozen on round creation; later root writes cannot replace them.
    let globals: std::collections::BTreeSet<_> = graph
        .global_variables()
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    pool.values.retain(|key, _| {
        pool.catalog.get(key).is_some_and(|decl| {
            let writer = &decl.writer;
            globals.contains(key.as_str())
                || graph.loop_owner(&pos.node_id) == Some(writer)
                || (graph.region_owner(&pos.node_id) == Some(writer)
                    && (key == &format!("{writer}.item") || key == &format!("{writer}.index")))
                || rows.iter().any(|row| {
                    row.node_id == *writer
                        && row.status == ora_domain::WorkflowNodeStatus::Succeeded
                        && ((row.scope_id.as_ref() == pos.scope_id && row.iteration == pos.index)
                            || (row.scope_id.as_ref() == pos.root_scope_id
                                && row.iteration.is_none()))
                })
        })
    });
    let completed: Vec<_> = scoped
        .iter()
        .filter(|n| n.status == ora_domain::WorkflowNodeStatus::Succeeded)
        .map(|n| n.id.as_ref())
        .collect();
    let mut proven_payload = payload;
    proven_payload.variable_pool = pool.clone();
    let entry = json!({"runPayload":serde_json::to_string(&proven_payload)?,"scopeState":json!({"variablePool":pool}).to_string(),"completedAttempts":completed});
    Ok((
        WorkflowDebugTarget {
            node_run_id: None,
            root_scope_id: Some(pos.root_scope_id.clone()),
            scope_id: pos.scope_id.clone(),
            node_id: pos.node_id.clone(),
            container_node_run_id: owner_id,
            round,
            index: pos.index,
        },
        Some(snapshot),
        Some(entry.to_string()),
        Some(json!({"variablePool":pool}).to_string()),
        kind.into(),
        raw,
    ))
}

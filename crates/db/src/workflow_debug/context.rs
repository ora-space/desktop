//! Resolves an execution position and replays its effective graph in a single database transaction.
use super::*;

pub(super) struct SelectedContext {
    pub target: WorkflowDebugTarget,
    pub entry: Value,
    pub dispatch: Option<Value>,
    pub kind: String,
    pub pool: WorkflowVariablePool,
    pub graph: Value,
    pub evidence_source: Option<String>,
}

/// Loads evidence and effective configuration for both the editor and the preview validator.
pub(super) fn load(
    db: &Connection,
    run: &str,
    selection: &WorkflowDebugSelection,
    revision: u32,
    confirmed_retry_source: Option<&str>,
) -> Result<SelectedContext, DatabaseError> {
    if let WorkflowDebugSelection::NodeRunId(id) = selection {
        // An attempt recorded before the current snapshot baseline keeps its inputs as history
        // only; the page has to select a position the current baseline has produced.
        let recorded = db
            .query_row(
                "SELECT e.revision,COALESCE((SELECT MAX(b.revision) FROM workflow_debug_baselines b
                    WHERE b.run_id=n.run_id),0)
                 FROM workflow_node_runs n JOIN workflow_debug_attempts e ON e.node_run_id=n.id
                 WHERE n.run_id=?1 AND n.id=?2 AND n.is_deleted=0",
                params![run, id],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?;
        if let Some((revision, baseline)) = recorded
            && revision < baseline
        {
            return Err(
                DebugRejection::MissingFact(WorkflowDebugReason::BeforeBaseline {
                    attempt: id.clone(),
                    revision,
                    baseline,
                })
                .into(),
            );
        }
    }
    let (target, snapshot, entry, dispatch, kind, graph_json, evidence_source) = match selection {
        WorkflowDebugSelection::PendingTarget(position) => super::pending::resolve(db, run, position)?,
        WorkflowDebugSelection::NodeRunId(id) => db.query_row(
        "SELECT n.node_id,n.scope_id,n.iteration,s.round_index,e.container_attempt,
                e.snapshot_id,entry.entry_state,CASE WHEN n.node_type='condition' AND json_valid(n.payload)
                    THEN COALESCE(condition.body,json_extract(n.payload,'$.debugCondition')) ELSE COALESCE(entry.dispatch_input, prior_entry.dispatch_input) END AS observed_input,n.node_type,snap.graph,n.id,CASE WHEN e.dispatch_input IS NULL THEN prior.node_run_id END AS evidence_source
         FROM workflow_node_runs n JOIN workflow_runs r ON r.id=n.run_id
         JOIN workflow_run_root_scopes root ON root.run_id=r.id
         JOIN workflow_execution_scopes s ON s.id=n.scope_id
         LEFT JOIN workflow_node_runs parent ON parent.id=s.parent_loop_node_run_id
          LEFT JOIN workflow_debug_restored_documents condition ON condition.node_run_id=n.id AND condition.kind='condition'
          LEFT JOIN workflow_debug_attempts e ON e.node_run_id=n.id
          LEFT JOIN workflow_debug_entries entry ON entry.node_run_id=n.id
          LEFT JOIN workflow_debug_attempts prior ON prior.node_run_id=COALESCE(?3, (
            SELECT candidate.id FROM json_each(CASE WHEN json_valid(n.payload) THEN n.payload END,'$.retry_chain') chain
            JOIN workflow_node_runs candidate ON candidate.id=chain.value
            JOIN workflow_debug_attempts evidence ON evidence.node_run_id=candidate.id
            WHERE n.status=1 AND n.started_at IS NULL AND json_extract(CASE WHEN json_valid(n.payload) THEN n.payload END,'$.retry_wait.previous_node_run_id') IS NOT NULL
              AND evidence.dispatch_input IS NOT NULL ORDER BY chain.key DESC LIMIT 1))
            AND prior.snapshot_id=e.snapshot_id AND prior.revision=e.revision AND prior.container_attempt IS e.container_attempt
            AND EXISTS (SELECT 1 FROM workflow_node_runs candidate WHERE candidate.id=prior.node_run_id
                AND candidate.is_deleted=1 AND candidate.status=3 AND candidate.run_id=n.run_id
                AND candidate.scope_id=n.scope_id AND candidate.node_id=n.node_id AND candidate.iteration IS n.iteration)
         LEFT JOIN workflow_debug_entries prior_entry ON prior_entry.node_run_id=prior.node_run_id
         JOIN workflow_snapshots snap ON snap.id=r.snapshot_id
         WHERE n.run_id=?1 AND n.id=?2 AND n.is_deleted=0
           AND (n.scope_id=root.scope_id OR (parent.scope_id=root.scope_id AND parent.is_deleted=0))
           AND (e.snapshot_id IS NULL OR e.snapshot_id=r.snapshot_id)",
        params![run, id, confirmed_retry_source],
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
    let pool: WorkflowVariablePool = serde_json::from_value(pool)?;
    let mut graph: Value = serde_json::from_str(&graph_json)?;
    configuration::replay(db, run, &target, &snapshot, revision, &mut graph)?;
    Ok(SelectedContext {
        target,
        entry,
        dispatch,
        kind,
        pool,
        graph,
        evidence_source,
    })
}

impl WorkflowDebugStore {
    /// Reads a position under the active revision without producing a preview receipt.
    pub fn edit_context(
        &self,
        request: &GetWorkflowDebugContextRequest,
    ) -> Result<GetWorkflowDebugContextResponse, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let tx = connection.transaction()?;
            fence::ensure_inactive(&tx, &request.run_id)?;
            let base_revision = run_version(&tx, &request.run_id)?;
            let selected = load(
                &tx,
                &request.run_id,
                &request.selection,
                base_revision,
                None,
            )?;
            Ok(GetWorkflowDebugContextResponse {
                base_revision,
                target: selected.target,
                graph: serde_json::to_string(&selected.graph)?,
            })
        })
    }
}

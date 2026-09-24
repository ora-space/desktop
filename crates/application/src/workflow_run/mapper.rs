use ora_contracts::{
    WorkflowExecutionScope as ContractExecutionScope,
    WorkflowExecutionScopeStatus as ContractExecutionScopeStatus,
    WorkflowNodeFailedAttempt as ContractFailedAttempt, WorkflowNodeRun as ContractNodeRun,
    WorkflowNodeStatus as ContractNodeStatus, WorkflowRun as ContractRun,
    WorkflowRunStatus as ContractRunStatus, WorkflowRunSummary as ContractRunSummary,
};
use ora_domain::{
    WorkflowExecutionScope, WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunStatus,
    WorkflowRunSummary, WorkflowScopeStatus,
};

/// Converts a domain run into its public contract representation.
pub(crate) fn map_run(run: WorkflowRun) -> ContractRun {
    ContractRun {
        id: run.id.to_string(),
        workspace_id: run.workspace_id.to_string(),
        workflow_id: run.workflow_id.to_string(),
        snapshot_id: run.snapshot_id.to_string(),
        name: run.name,
        status: map_run_status(run.status),
        state: run.state,
        input: run.input,
        output: run.output,
        error: run.error,
        started_at: run.started_at,
        finished_at: run.finished_at,
        created_at: run.audit_fields.created_at,
        updated_at: run.audit_fields.updated_at,
    }
}

/// Converts a domain run into its public contract representation, deriving `AwaitingInput` when
/// the run is `Running` and has an awaiting (`Pending`) interactive node.
pub(crate) fn map_run_awaiting(run: WorkflowRun, has_awaiting_node: bool) -> ContractRun {
    let awaiting = run.status == WorkflowRunStatus::Running && has_awaiting_node;
    let mut mapped = map_run(run);
    if awaiting {
        mapped.status = ContractRunStatus::AwaitingInput;
    }
    mapped
}

/// Converts a domain node run into its public contract representation.
pub(crate) fn map_node_run(node_run: WorkflowNodeRun) -> ContractNodeRun {
    ContractNodeRun {
        id: node_run.id.to_string(),
        run_id: node_run.run_id.to_string(),
        scope_id: node_run.scope_id.to_string(),
        node_id: node_run.node_id,
        node_type: node_run.node_type,
        session_id: node_run.session_id.map(|id| id.to_string()),
        status: map_node_status(node_run.status),
        input: node_run.input,
        output: node_run.output,
        error: node_run.error,
        payload: node_run.payload,
        iteration: node_run.iteration,
        started_at: node_run.started_at,
        finished_at: node_run.finished_at,
        created_at: node_run.audit_fields.created_at,
        updated_at: node_run.audit_fields.updated_at,
    }
}

/// Converts one soft-deleted failed attempt into an attempt-history entry. Rows without a
/// readable `payload.error_detail` (written before failure details existed) are skipped.
pub(crate) fn map_failed_attempt(node_run: WorkflowNodeRun) -> Option<ContractFailedAttempt> {
    let mut payload: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(node_run.payload.as_deref()?).ok()?;
    let detail: crate::NodeFailureDetail =
        serde_json::from_value(payload.remove("error_detail")?).ok()?;
    Some(ContractFailedAttempt {
        node_run_id: node_run.id.to_string(),
        node_id: node_run.node_id,
        scope_id: node_run.scope_id.to_string(),
        iteration: node_run.iteration,
        session_id: node_run.session_id.map(|id| id.to_string()),
        attempt: detail.attempt,
        kind: detail.kind.as_str().to_string(),
        message: detail.message,
        source_chain: detail.source_chain,
        recorded_at: detail.recorded_at,
        started_at: node_run.started_at,
        finished_at: node_run.finished_at,
    })
}

/// Converts one internal Loop round identity into its history contract.
pub(crate) fn map_execution_scope(scope: WorkflowExecutionScope) -> ContractExecutionScope {
    ContractExecutionScope {
        id: scope.id.to_string(),
        run_id: scope.run_id.to_string(),
        parent_loop_node_run_id: scope.parent_loop_node_run_id.to_string(),
        round_index: scope.round_index,
        status: match scope.status {
            WorkflowScopeStatus::Pending => ContractExecutionScopeStatus::Pending,
            WorkflowScopeStatus::Running => ContractExecutionScopeStatus::Running,
            WorkflowScopeStatus::Succeeded => ContractExecutionScopeStatus::Succeeded,
            WorkflowScopeStatus::Failed => ContractExecutionScopeStatus::Failed,
            WorkflowScopeStatus::Cancelled => ContractExecutionScopeStatus::Cancelled,
        },
        created_at: scope.created_at,
        updated_at: scope.updated_at,
    }
}

/// Converts a domain run summary into its public contract representation.
pub(crate) fn map_run_summary(summary: WorkflowRunSummary) -> ContractRunSummary {
    ContractRunSummary {
        id: summary.id.to_string(),
        name: summary.name,
        workspace_id: summary.workspace_id.to_string(),
        project_id: summary.project_id.to_string(),
        workflow_id: summary.workflow_id.to_string(),
        version: summary.version,
        status: map_summary_status(summary.status, summary.has_awaiting_node),
        started_at: summary.started_at,
        finished_at: summary.finished_at,
        created_at: summary.created_at,
    }
}

/// Derives the summary's wire status: a `Running` run with an awaiting node reads as
/// `AwaitingInput` so the sidebar surfaces the need for human action.
fn map_summary_status(status: WorkflowRunStatus, has_awaiting_node: bool) -> ContractRunStatus {
    if status == WorkflowRunStatus::Running && has_awaiting_node {
        ContractRunStatus::AwaitingInput
    } else {
        map_run_status(status)
    }
}

/// Translates the internal run status into the transport-facing enum.
fn map_run_status(status: WorkflowRunStatus) -> ContractRunStatus {
    match status {
        WorkflowRunStatus::Pending => ContractRunStatus::Pending,
        WorkflowRunStatus::Running => ContractRunStatus::Running,
        WorkflowRunStatus::Succeeded => ContractRunStatus::Succeeded,
        WorkflowRunStatus::Failed => ContractRunStatus::Failed,
        WorkflowRunStatus::Cancelled => ContractRunStatus::Cancelled,
    }
}

/// Translates the internal node status into the transport-facing enum.
fn map_node_status(status: WorkflowNodeStatus) -> ContractNodeStatus {
    match status {
        WorkflowNodeStatus::Pending => ContractNodeStatus::Pending,
        WorkflowNodeStatus::Running => ContractNodeStatus::Running,
        WorkflowNodeStatus::Succeeded => ContractNodeStatus::Succeeded,
        WorkflowNodeStatus::Failed => ContractNodeStatus::Failed,
        WorkflowNodeStatus::Cancelled => ContractNodeStatus::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::{map_failed_attempt, map_run_awaiting, map_run_summary};
    use ora_contracts::{
        WorkflowNodeFailedAttempt as ContractFailedAttempt, WorkflowRunStatus as ContractRunStatus,
    };
    use ora_domain::{
        AuditFields, ProjectId, SessionId, WorkflowId, WorkflowNodeRun, WorkflowNodeRunId,
        WorkflowNodeStatus, WorkflowRun, WorkflowRunId, WorkflowRunStatus, WorkflowRunSummary,
        WorkflowScopeId, WorkflowSnapshotId, WorkspaceId,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn running_run() -> WorkflowRun {
        WorkflowRun::new(
            WorkflowRunId::new("run-1"),
            WorkspaceId::new("workspace-1"),
            WorkflowId::new("wf-1"),
            WorkflowSnapshotId::new("snap-1"),
            "run",
            WorkflowRunStatus::Running,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            AuditFields::new(1, 1, false),
        )
    }

    /// A `Running` run reads as `AwaitingInput` on the detail wire only when it has an awaiting
    /// node; otherwise it keeps its plain status.
    #[test]
    fn map_run_awaiting_derives_awaiting_input_only_with_an_awaiting_node() {
        assert_eq!(
            map_run_awaiting(running_run(), true).status,
            ContractRunStatus::AwaitingInput
        );
        assert_eq!(
            map_run_awaiting(running_run(), false).status,
            ContractRunStatus::Running
        );
    }

    /// A listed summary derives `AwaitingInput` for the sidebar from the awaiting-node flag.
    #[test]
    fn map_run_summary_derives_awaiting_input_for_listing() {
        let summary = WorkflowRunSummary {
            id: WorkflowRunId::new("run-1"),
            name: "run".to_string(),
            workspace_id: WorkspaceId::new("workspace-1"),
            project_id: ProjectId::new("project-1"),
            workflow_id: WorkflowId::new("wf-1"),
            version: "v1".to_string(),
            status: WorkflowRunStatus::Running,
            has_awaiting_node: true,
            started_at: None,
            finished_at: None,
            created_at: 1,
        };
        assert_eq!(
            map_run_summary(summary.clone()).status,
            ContractRunStatus::AwaitingInput
        );
        assert_eq!(
            map_run_summary(WorkflowRunSummary {
                has_awaiting_node: false,
                ..summary
            })
            .status,
            ContractRunStatus::Running
        );
    }

    fn failed_row(payload: Option<String>) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new("node-run-1"),
            WorkflowRunId::new("run-1"),
            WorkflowScopeId::new("scope-1"),
            "worker",
            "agent",
            Some(SessionId::new("session-1")),
            WorkflowNodeStatus::Failed,
            None,
            None,
            Some("session failed".to_string()),
            payload,
            Some(10),
            Some(20),
            AuditFields::new(10, 20, true),
        )
        .in_iteration(Some(2))
    }

    /// An attempt-history entry carries the failure's source chain in order, because the
    /// top-level message of a session failure is generic and the agent's reason is in the chain.
    #[test]
    fn map_failed_attempt_carries_the_source_chain_outermost_first() {
        let payload = json!({
            "error_detail": {
                "kind": "session",
                "message": "session failed: agent protocol operation failed",
                "source_chain": ["agent protocol operation failed", "model overloaded (529)"],
                "attempt": 2,
                "resumable": true,
                "injects_previous_failure": false,
                "recorded_at": 20
            }
        });
        assert_eq!(
            map_failed_attempt(failed_row(Some(payload.to_string()))),
            Some(ContractFailedAttempt {
                node_run_id: "node-run-1".to_string(),
                node_id: "worker".to_string(),
                scope_id: "scope-1".to_string(),
                iteration: Some(2),
                session_id: Some("session-1".to_string()),
                attempt: 2,
                kind: "session".to_string(),
                message: "session failed: agent protocol operation failed".to_string(),
                source_chain: vec![
                    "agent protocol operation failed".to_string(),
                    "model overloaded (529)".to_string()
                ],
                recorded_at: 20,
                started_at: Some(10),
                finished_at: Some(20),
            })
        );
    }

    /// Rows without a readable failure detail are not attempt history.
    #[test]
    fn map_failed_attempt_skips_rows_without_a_failure_detail() {
        assert_eq!(map_failed_attempt(failed_row(None)), None);
        assert_eq!(
            map_failed_attempt(failed_row(Some(json!({"other": 1}).to_string()))),
            None
        );
        assert_eq!(
            map_failed_attempt(failed_row(Some("not json".to_string()))),
            None
        );
    }
}

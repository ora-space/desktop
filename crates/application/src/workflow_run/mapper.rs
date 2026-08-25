use ora_contracts::{
    WorkflowNodeRun as ContractNodeRun, WorkflowNodeStatus as ContractNodeStatus,
    WorkflowRun as ContractRun, WorkflowRunStatus as ContractRunStatus,
    WorkflowRunSummary as ContractRunSummary,
};
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunStatus, WorkflowRunSummary,
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
        payload: run.payload,
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
        node_id: node_run.node_id,
        node_type: node_run.node_type,
        session_id: node_run.session_id.map(|id| id.to_string()),
        status: map_node_status(node_run.status),
        input: node_run.input,
        output: node_run.output,
        error: node_run.error,
        payload: node_run.payload,
        started_at: node_run.started_at,
        finished_at: node_run.finished_at,
        created_at: node_run.audit_fields.created_at,
        updated_at: node_run.audit_fields.updated_at,
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
    use super::{map_run_awaiting, map_run_summary};
    use ora_contracts::WorkflowRunStatus as ContractRunStatus;
    use ora_domain::{
        AuditFields, ProjectId, WorkflowId, WorkflowRun, WorkflowRunId, WorkflowRunStatus,
        WorkflowRunSummary, WorkflowSnapshotId, WorkspaceId,
    };
    use pretty_assertions::assert_eq;

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
}

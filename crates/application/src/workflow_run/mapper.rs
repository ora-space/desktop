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
        workflow_id: run.workflow_id.to_string(),
        snapshot_id: run.snapshot_id.to_string(),
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
        project_id: summary.project_id.to_string(),
        workflow_id: summary.workflow_id.to_string(),
        status: map_run_status(summary.status),
        started_at: summary.started_at,
        finished_at: summary.finished_at,
        created_at: summary.created_at,
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

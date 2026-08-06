use ora_domain::{
    CreatedWorkflow, Workflow, WorkflowDetail, WorkflowSnapshot, WorkflowSummary, WorkflowVersion,
};

/// Converts a domain workflow into its public contract representation.
pub(crate) fn map_workflow(workflow: Workflow) -> ora_contracts::Workflow {
    ora_contracts::Workflow {
        id: workflow.id.to_string(),
        name: workflow.name,
        published_snapshot_id: workflow.published_snapshot_id.map(|id| id.to_string()),
        created_at: workflow.audit_fields.created_at,
        updated_at: workflow.audit_fields.updated_at,
    }
}

/// Converts a domain snapshot into its public contract representation.
pub(crate) fn map_snapshot(snapshot: WorkflowSnapshot) -> ora_contracts::WorkflowSnapshot {
    ora_contracts::WorkflowSnapshot {
        id: snapshot.id.to_string(),
        workflow_id: snapshot.workflow_id.to_string(),
        version: snapshot.version,
        graph: snapshot.graph,
        created_at: snapshot.created_at,
        updated_at: snapshot.updated_at,
    }
}

/// Converts a domain workflow summary into its public contract representation.
pub(crate) fn map_workflow_summary(summary: WorkflowSummary) -> ora_contracts::WorkflowSummary {
    ora_contracts::WorkflowSummary {
        id: summary.id,
        name: summary.name,
        published_version: summary.published_version,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
    }
}

/// Converts a domain workflow version into its public contract representation.
pub(crate) fn map_workflow_version(version: WorkflowVersion) -> ora_contracts::WorkflowVersion {
    ora_contracts::WorkflowVersion {
        id: version.id,
        version: version.version,
        created_at: version.created_at,
    }
}

/// Converts a created workflow result into its public contract representation.
pub(crate) fn map_created_workflow(
    created: CreatedWorkflow,
) -> ora_contracts::CreateWorkflowResponse {
    ora_contracts::CreateWorkflowResponse {
        workflow: map_workflow(created.workflow),
        draft: map_snapshot(created.draft),
    }
}

/// Converts a workflow detail result into its public contract representation.
pub(crate) fn map_workflow_detail(detail: WorkflowDetail) -> ora_contracts::GetWorkflowResponse {
    ora_contracts::GetWorkflowResponse {
        workflow: map_workflow(detail.workflow),
        draft: map_snapshot(detail.draft),
        published: detail.published.map(map_snapshot),
    }
}

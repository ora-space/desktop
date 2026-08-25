use crate::{AuditFields, ProjectId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Represents a logical unit of work inside a project.
///
/// Tasks do not carry a kanban status: session rows show conversation activity,
/// and workflow-run progress lives on `WorkflowRunStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: crate::TaskId,
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub audit_fields: AuditFields,
}

impl Task {
    /// Creates the user-facing label for a newly provisioned worktree workspace.
    pub fn new(
        id: crate::TaskId,
        project_id: ProjectId,
        workspace_id: WorkspaceId,
        title: impl Into<String>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            workspace_id,
            title: title.into(),
            audit_fields,
        }
    }
}

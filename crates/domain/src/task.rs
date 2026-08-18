use crate::{AuditFields, DomainModelError, ProjectId, WorkflowRunId, WorktreeId};
use serde::{Deserialize, Serialize};

/// Captures the task kind so the frontend can tell ordinary tasks from workflow runs
/// without resolving run detail.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskType {
    /// An ordinary task created through the generic task path.
    #[default]
    Default,
    /// A task that backs one workflow run and shares its lifecycle.
    Workflow,
}

impl TaskType {
    /// Returns the integer code used by persistence adapters for this task type.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Default => 0,
            Self::Workflow => 1,
        }
    }

    /// Converts a persisted integer into a strongly typed task type.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Default),
            1 => Ok(Self::Workflow),
            _ => Err(DomainModelError::InvalidTaskType(value)),
        }
    }
}

impl TryFrom<i64> for TaskType {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents a logical unit of work inside a project.
///
/// Tasks do not carry a kanban status: session rows show conversation activity,
/// and workflow-run progress lives on `WorkflowRunStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: crate::TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub task_type: TaskType,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub worktree_id: Option<WorktreeId>,
    pub audit_fields: AuditFields,
}

impl Task {
    /// Creates an ordinary task snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: crate::TaskId,
        project_id: ProjectId,
        title: impl Into<String>,
        worktree_id: Option<WorktreeId>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            title: title.into(),
            task_type: TaskType::Default,
            workflow_run_id: None,
            worktree_id,
            audit_fields,
        }
    }

    /// Creates a workflow-run task that fixes `task_type=WorkflowRun` and requires a run id
    /// plus a dedicated worktree id, keeping run-task construction self-documenting.
    pub fn workflow_run(
        id: crate::TaskId,
        project_id: ProjectId,
        title: impl Into<String>,
        workflow_run_id: WorkflowRunId,
        worktree_id: WorktreeId,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            title: title.into(),
            task_type: TaskType::Workflow,
            workflow_run_id: Some(workflow_run_id),
            worktree_id: Some(worktree_id),
            audit_fields,
        }
    }
}

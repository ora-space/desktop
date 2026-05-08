use crate::{AuditFields, DomainModelError, ProjectId, WorktreeId};
use serde::{Deserialize, Serialize};

/// Captures the lifecycle state for a task without exposing database integer codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Todo,
    Doing,
    Done,
}

impl TaskStatus {
    /// Returns the integer code used by persistence adapters for this task status.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Todo => 0,
            Self::Doing => 1,
            Self::Done => 2,
        }
    }

    /// Converts a persisted integer into a strongly typed task status.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Todo),
            1 => Ok(Self::Doing),
            2 => Ok(Self::Done),
            _ => Err(DomainModelError::InvalidTaskStatus(value)),
        }
    }
}

impl TryFrom<i64> for TaskStatus {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents a logical unit of work inside a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: crate::TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub status: TaskStatus,
    pub worktree_id: Option<WorktreeId>,
    pub audit_fields: AuditFields,
}

impl Task {
    /// Creates a task snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: crate::TaskId,
        project_id: ProjectId,
        title: impl Into<String>,
        status: TaskStatus,
        worktree_id: Option<WorktreeId>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            project_id,
            title: title.into(),
            status,
            worktree_id,
            audit_fields,
        }
    }
}

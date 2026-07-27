use crate::{AuditFields, DomainModelError, TaskId};
use serde::{Deserialize, Deserializer, Serialize};

/// Represents an optional immutable baseline without exposing an empty recorded state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeBaseline(Option<String>);

impl WorktreeBaseline {
    /// Creates a recorded baseline only when a Git object identifier was actually supplied.
    pub fn recorded(commit_id: impl Into<String>) -> Result<Self, DomainModelError> {
        let commit_id = commit_id.into();
        if commit_id.trim().is_empty() {
            return Err(DomainModelError::EmptyWorktreeBaseline);
        }
        Ok(Self(Some(commit_id)))
    }

    /// Represents historical worktrees whose creation commit was never recorded.
    pub fn unavailable() -> Self {
        Self(None)
    }

    /// Returns the recorded commit identifier when this worktree supports stable diffs.
    pub fn commit_id(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl<'de> Deserialize<'de> for WorktreeBaseline {
    /// Reuses the validated constructor so deserialization cannot create an empty baseline.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<String>::deserialize(deserializer)? {
            Some(commit_id) => Self::recorded(commit_id).map_err(serde::de::Error::custom),
            None => Ok(Self::unavailable()),
        }
    }
}

/// Models whether a worktree is the active working copy for its task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeActivity {
    Inactive,
    Active,
}

impl WorktreeActivity {
    /// Returns the integer code used by persistence adapters for this activity value.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Inactive => 0,
            Self::Active => 1,
        }
    }

    /// Converts a persisted integer into a strongly typed worktree activity value.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Inactive),
            1 => Ok(Self::Active),
            _ => Err(DomainModelError::InvalidWorktreeActivity(value)),
        }
    }
}

impl TryFrom<i64> for WorktreeActivity {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents the physical git worktree that backs a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub id: crate::WorktreeId,
    pub task_id: TaskId,
    pub branch_name: Option<String>,
    pub baseline: WorktreeBaseline,
    pub activity: WorktreeActivity,
    pub audit_fields: AuditFields,
}

impl Worktree {
    /// Creates a worktree snapshot together with its persistence-managed audit metadata.
    pub fn new(
        id: crate::WorktreeId,
        task_id: TaskId,
        branch_name: Option<String>,
        baseline: WorktreeBaseline,
        activity: WorktreeActivity,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            branch_name,
            baseline,
            activity,
            audit_fields,
        }
    }
}

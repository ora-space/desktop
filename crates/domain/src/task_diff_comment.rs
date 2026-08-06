use crate::{AuditFields, TaskDiffCommentId, TaskId};
use serde::{Deserialize, Serialize};

/// Identifies which side of a two-way diff owns a line comment anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskDiffSide {
    Old,
    New,
}

impl TaskDiffSide {
    /// Returns the integer code used by persistence adapters for this diff side.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Old => 0,
            Self::New => 1,
        }
    }

    /// Converts a persisted integer into a strongly typed diff side.
    pub fn from_database_value(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Old),
            1 => Some(Self::New),
            _ => None,
        }
    }
}

/// Captures whether a root diff discussion still requires attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskDiffThreadStatus {
    Open,
    Resolved,
}

impl TaskDiffThreadStatus {
    /// Returns the integer code used by persistence adapters for this thread state.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Open => 0,
            Self::Resolved => 1,
        }
    }

    /// Converts a persisted integer into a strongly typed thread state.
    pub fn from_database_value(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Open),
            1 => Some(Self::Resolved),
            _ => None,
        }
    }
}

/// Anchors a root discussion to a stable diff snapshot and source line range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDiffAnchor {
    pub diff_id: String,
    pub path: String,
    pub side: TaskDiffSide,
    pub start_line: u32,
    pub end_line: u32,
    pub hunk_header: String,
    pub line_content: String,
}

/// Distinguishes root line discussions from replies so invalid partial anchors cannot be constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskDiffCommentKind {
    Thread {
        anchor: TaskDiffAnchor,
        status: TaskDiffThreadStatus,
    },
    Reply {
        parent_comment_id: TaskDiffCommentId,
    },
}

/// Represents one persisted message in a task diff discussion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDiffComment {
    pub id: TaskDiffCommentId,
    pub task_id: TaskId,
    pub kind: TaskDiffCommentKind,
    pub body: String,
    pub audit_fields: AuditFields,
}

impl TaskDiffComment {
    /// Creates one root discussion or reply with persistence-managed audit metadata.
    pub fn new(
        id: TaskDiffCommentId,
        task_id: TaskId,
        kind: TaskDiffCommentKind,
        body: impl Into<String>,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            kind,
            body: body.into(),
            audit_fields,
        }
    }
}

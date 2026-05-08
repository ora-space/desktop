use crate::{AuditFields, DomainModelError, TaskId};
use serde::{Deserialize, Serialize};

/// Captures whether an agent session is currently running or stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Running,
    Stopped,
}

impl SessionStatus {
    /// Returns the integer code used by persistence adapters for this session status.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Running => 0,
            Self::Stopped => 1,
        }
    }

    /// Converts a persisted integer into a strongly typed session status.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Running),
            1 => Ok(Self::Stopped),
            _ => Err(DomainModelError::InvalidSessionStatus(value)),
        }
    }
}

impl TryFrom<i64> for SessionStatus {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents one historical run of an agent for a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: crate::SessionId,
    pub task_id: TaskId,
    pub agent_id: String,
    pub agent_session_id: Option<String>,
    pub status: SessionStatus,
    pub audit_fields: AuditFields,
}

impl Session {
    /// Creates a session snapshot that keeps both the logical agent identity and any provider session id.
    pub fn new(
        id: crate::SessionId,
        task_id: TaskId,
        agent_id: impl Into<String>,
        agent_session_id: Option<String>,
        status: SessionStatus,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            agent_id: agent_id.into(),
            agent_session_id,
            status,
            audit_fields,
        }
    }
}

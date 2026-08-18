use crate::{AgentRef, AuditFields, DomainModelError, TaskId};
use serde::{Deserialize, Serialize};

/// Captures whether a conversation is registered on its shared CLI connection.
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

/// Reports whether Ora can still extend this session's durable history.
///
/// This is deliberately independent of [`SessionStatus`]. That describes whether
/// the conversation is registered on a CLI connection; this describes whether the
/// record of it can still grow. A session can be registered and unwritable at the
/// same time, which is exactly what happens when the disk fills mid-turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryState {
    /// History is being recorded normally.
    Writable,
    /// A write failed, so the session refuses further prompts until it is resumed.
    ///
    /// The reason is carried rather than logged and forgotten because the user has
    /// to be told what to fix, and because resuming records it as the explanation
    /// for the gap the failure left in the file.
    Degraded { reason: String },
}

impl HistoryState {
    /// Returns the persisted reason column, where absence means writable.
    ///
    /// Storing one nullable column rather than a flag and a reason keeps
    /// "degraded with no explanation" out of the database entirely.
    pub fn database_value(&self) -> Option<&str> {
        match self {
            Self::Writable => None,
            Self::Degraded { reason } => Some(reason),
        }
    }

    /// Restores the state from its persisted reason column.
    pub fn from_database_value(reason: Option<String>) -> Self {
        match reason {
            Some(reason) => Self::Degraded { reason },
            None => Self::Writable,
        }
    }
}

/// Represents one conversation and the provider session it currently runs on.
///
/// `agent_ref` and `agent_session_id` are the conversation's *current* binding,
/// not its identity. Switching agents replaces both while the conversation, its
/// identifier, and its history file all continue unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: crate::SessionId,
    pub task_id: TaskId,
    pub agent_ref: AgentRef,
    pub agent_session_id: String,
    pub title: Option<crate::SessionTitle>,
    pub status: SessionStatus,
    pub history_state: HistoryState,
    pub audit_fields: AuditFields,
}

impl Session {
    /// Creates a session only after the provider has returned its required session identifier.
    ///
    /// A new session always starts writable; a degraded one can only be produced
    /// by a failed write or restored from storage.
    pub fn new(
        id: crate::SessionId,
        task_id: TaskId,
        agent_ref: AgentRef,
        agent_session_id: impl Into<String>,
        status: SessionStatus,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            agent_ref,
            agent_session_id: agent_session_id.into(),
            title: None,
            status,
            history_state: HistoryState::Writable,
            audit_fields,
        }
    }

    /// Changes only registration state while preserving the current provider binding.
    pub fn with_status(mut self, status: SessionStatus, updated_at: i64) -> Self {
        self.status = status;
        self.audit_fields.updated_at = updated_at;
        self
    }

    /// Replaces the persisted display title without changing audit timestamps during restoration.
    pub fn with_title(mut self, title: Option<crate::SessionTitle>) -> Self {
        self.title = title;
        self
    }

    /// Points this conversation at a different provider session, possibly on another agent.
    ///
    /// The identifier and task stay fixed, so the history file the conversation
    /// owns is unaffected by the move.
    pub fn with_binding(
        mut self,
        agent_ref: AgentRef,
        agent_session_id: impl Into<String>,
        updated_at: i64,
    ) -> Self {
        self.agent_ref = agent_ref;
        self.agent_session_id = agent_session_id.into();
        self.audit_fields.updated_at = updated_at;
        self
    }

    /// Replaces whether this session's history can still be extended.
    pub fn with_history_state(mut self, history_state: HistoryState, updated_at: i64) -> Self {
        self.history_state = history_state;
        self.audit_fields.updated_at = updated_at;
        self
    }

    /// Restores the persisted history state without disturbing audit timestamps.
    ///
    /// Reconstruction from storage is not a change to the session, so it must not
    /// look like one.
    pub fn restoring_history_state(mut self, history_state: HistoryState) -> Self {
        self.history_state = history_state;
        self
    }
}

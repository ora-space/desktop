//! Stored commands keep their full wire identities; the application adapts them to runtime commands.
use ora_node_protocol::*;
use serde::{Deserialize, Serialize};

/// Original session input and retained terminal state, independent of event acknowledgements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionExecution {
    /// The input without runtime-only values: MCP settings are empty and there are no host MCP
    /// servers, whose names alone are kept below.
    pub command: StartAgentSessionMessage,
    /// Names of the host MCP servers the session started with, in input order. URLs and headers
    /// are runtime values and never reach the database.
    pub host_mcp_server_names: Vec<String>,
    pub state: ExecutionState,
    /// Zero before the first event; retained even when all outbox entries are acknowledged.
    pub last_sequence: u64,
}

/// A durable command's mutually exclusive settlement states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionCommandState {
    Failed,
    Queued,
    Executed,
    Discarded,
}

/// A settlement cannot return a command to the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionCommandSettlement {
    Executed,
    Discarded,
}

/// Terminal sessions reject commands without recording a new queue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAdmission {
    McpUpdateRequired,
    Accepted,
    SessionEnded,
    /// `CancelTurn` for a session that did not choose `keep_session_on_cancel`; nothing stored.
    TurnCancelUnsupported,
}

/// Immutable command input, including its execution, operation and target identities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "command", rename_all = "snake_case")]
pub enum SessionCommandInput {
    SubmitUserTurn(SubmitUserTurnMessage),
    EndSession(EndSessionMessage),
    CancelTurn(CancelTurnMessage),
}

impl SessionCommandInput {
    /// Names the immutable command used for deduplication within its execution.
    pub fn command_id(&self) -> &CommandId {
        match self {
            Self::SubmitUserTurn(m) => &m.payload.command_id,
            Self::EndSession(m) => &m.payload.command_id,
            Self::CancelTurn(m) => &m.payload.command_id,
        }
    }
    /// Names the session that owns this queue entry.
    pub fn execution_id(&self) -> &ExecutionId {
        match self {
            Self::SubmitUserTurn(m) => &m.execution_id,
            Self::EndSession(m) => &m.execution_id,
            Self::CancelTurn(m) => &m.execution_id,
        }
    }
    /// Preserves the operation identity rather than accepting an execution-only alias.
    pub fn operation_id(&self) -> &OperationId {
        match self {
            Self::SubmitUserTurn(m) => &m.operation_id,
            Self::EndSession(m) => &m.operation_id,
            Self::CancelTurn(m) => &m.operation_id,
        }
    }
    /// Identifies the only Node permitted to admit the command.
    pub fn node_id(&self) -> &NodeId {
        match self {
            Self::SubmitUserTurn(m) => &m.payload.node_id,
            Self::EndSession(m) => &m.payload.node_id,
            Self::CancelTurn(m) => &m.payload.node_id,
        }
    }
}
impl ValidateMessage for SessionCommandInput {
    /// Uses the same validation before persistence as the eventual wire entrance.
    fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::SubmitUserTurn(m) => m.validate(),
            Self::EndSession(m) => m.validate(),
            Self::CancelTurn(m) => m.validate(),
        }
    }
}

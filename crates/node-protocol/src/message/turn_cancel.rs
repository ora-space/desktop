//! Cancelling one turn of a running session without ending the session, and the settlement every
//! accepted cancel receives.
//!
//! `CancelTurn` is a session command like `SubmitUserTurn` and `EndSession`: it is durably accepted,
//! deduplicated by `command_id`, and answered with `SessionCommandAccepted`. Acceptance says nothing
//! about the turn; the one `TurnCancelSettled` event does. That event shares the execution's
//! sequence space with Thread events and `SessionTurnFailed`, so it is persisted before it is sent,
//! replayed after takeover, and acknowledged exactly. It is not a Thread record.

use super::validation::{validate_execution_ids, validate_identity, validate_protocol_version};
use crate::{
    CommandId, ExecutionId, MessageValidationError, NodeId, OperationId, ProtocolVersion, Sequence,
    TurnId, ValidateMessage,
};
use serde::{Deserialize, Serialize};

/// Cancels the turn named by `turn_id`; other turns and the session itself are untouched.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancelTurn {
    pub node_id: NodeId,
    pub command_id: CommandId,
    /// Any turn of the session, including the initial turn of `StartAgentSession`; naming the turn
    /// rather than "the current one" keeps a late stop from landing on the next turn.
    pub turn_id: TurnId,
}

/// What one accepted `CancelTurn` achieved.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCancelOutcome {
    /// The turn stopped because of the cancel and the session stays open for the next turn.
    Stopped,
    /// The turn was dropped before admission; the Agent never received it.
    NotStarted,
    /// The turn already had its own result before the cancel took effect; nothing was done.
    AlreadyEnded,
    /// The session never accepted this turn.
    Unknown,
    /// The session will not be kept. Not a terminal and not evidence that the Agent stopped: only
    /// `AgentSessionEnded` and `GetExecutionStatus` say that.
    SessionEnding,
}

/// The single settlement of one accepted `CancelTurn`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TurnCancelSettled {
    pub command_id: CommandId,
    pub turn_id: TurnId,
    pub outcome: TurnCancelOutcome,
}

/// Addresses a cancel to a running session execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelTurnMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: CancelTurn,
}

/// One sequenced, non-terminal settlement of a session execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnCancelSettledMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: TurnCancelSettled,
}

impl ValidateMessage for CancelTurnMessage {
    /// Requires the command and turn identities the settlement will echo.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.node_id.is_empty(), "node_id")?;
        validate_identity(self.payload.command_id.is_empty(), "command_id")?;
        validate_identity(self.payload.turn_id.is_empty(), "turn_id")
    }
}

impl ValidateMessage for TurnCancelSettledMessage {
    /// A settlement occupies its own reliable sequence and names the command it settles.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.sequence.value() == 0, "sequence")?;
        validate_identity(self.payload.command_id.is_empty(), "command_id")?;
        validate_identity(self.payload.turn_id.is_empty(), "turn_id")
    }
}

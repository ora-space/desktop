//! Agent session execution: its start command, non-terminal Thread events, terminal event, and
//! the session commands a running session accepts.
//!
//! Thread events and the terminal event share one gap-free sequence space per execution and follow
//! the same persist-before-send, acknowledge-after-takeover and replay rules as every other event.
//! Session commands and their replies carry no sequence: a lost reply is recovered by resending the
//! command, which a Node deduplicates by `command_id`.

use super::validation::{validate_execution_ids, validate_identity, validate_protocol_version};
use crate::{
    AgentSessionResult, AgentSessionSpec, CommandId, ExecutionId, MessageValidationError, NodeId,
    OperationId, ProtocolVersion, Sequence, ThreadEvent, UserTurn, ValidateMessage,
};
use serde::{Deserialize, Serialize};

/// Starts the agent plugin and sends the first user turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartAgentSession {
    pub spec: AgentSessionSpec,
}

/// Queues a user turn behind the one in progress; it never interrupts the current turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitUserTurn {
    /// Required for lifecycle sessions; a floor never asks Node to restore an old target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_generation: Option<u64>,
    pub node_id: NodeId,
    pub command_id: CommandId,
    pub turn: UserTurn,
}

/// Why Cloud asked a session to end.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndSessionReason {
    UserEnded,
    IdleTimeout,
    Cancelled,
}

/// Cancels the current turn, discards queued turns, stops the Agent and ends the execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndSession {
    pub node_id: NodeId,
    pub command_id: CommandId,
    pub reason: EndSessionReason,
}

/// The Node durably accepted a session command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCommandAccepted {
    pub command_id: CommandId,
}

/// Why a Node refused a session command without executing it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCommandRejection {
    /// The session already has a terminal result; Cloud settles by that result.
    SessionEnded,
    /// Retry the unchanged command after delivering its required configuration generation.
    McpUpdateRequired,
    /// `CancelTurn` for a session that did not choose `keep_session_on_cancel`; nothing changed.
    TurnCancelUnsupported,
}

/// Only terminal refusals are delivered; MCP update requirements leave the command retryable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCommandRejected {
    pub command_id: CommandId,
    pub reason: SessionCommandRejection,
}

/// Reliable failure of one admitted turn, separate from Agent-generated Thread records.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionTurnFailed {
    pub turn_id: crate::TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<CommandId>,
    pub code: SessionTurnFailureCode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionTurnFailureCode {
    McpRefreshWaitTimeout,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionTurnFailedMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: SessionTurnFailed,
}

impl ValidateMessage for SessionTurnFailedMessage {
    /// A failure has its own reliable sequence and only value-free bounded identities.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.sequence.value() == 0, "sequence")?;
        validate_identity(self.payload.turn_id.is_empty(), "turn_id")?;
        if let Some(command) = &self.payload.command_id {
            validate_identity(command.is_empty(), "command_id")?;
        }
        Ok(())
    }
}

/// Correlates a session with its IssueRun and durable execution identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StartAgentSessionMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: StartAgentSession,
}

/// One non-terminal event of a session execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ThreadEventMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: ThreadEvent,
}

/// The terminal event of a session execution; no event of the execution follows it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSessionEndedMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: AgentSessionResult,
}

/// Addresses a user turn to a running session execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubmitUserTurnMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: SubmitUserTurn,
}

/// Addresses an end request to a running session execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EndSessionMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: EndSession,
}

/// Reply to a session command; not an event, so it has no sequence and needs no acknowledgement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionCommandAcceptedMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: SessionCommandAccepted,
}

/// Reply to a session command; not an event, so it has no sequence and needs no acknowledgement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionCommandRejectedMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: SessionCommandRejected,
}

impl ValidateMessage for StartAgentSessionMessage {
    /// Rejects inputs the Node could not start deterministically.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        self.payload.spec.validate()
    }
}

impl ValidateMessage for ThreadEventMessage {
    /// Bounds the record so each event fits Cloud's Thread entry limit.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        self.payload.validate()
    }
}

impl ValidateMessage for AgentSessionEndedMessage {
    /// Applies session-owned terminal checks before event delivery or replay.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        self.payload.validate()
    }
}

impl ValidateMessage for SubmitUserTurnMessage {
    /// Requires command identity and a bounded turn.
    fn validate(&self) -> Result<(), MessageValidationError> {
        if self
            .payload
            .mcp_generation
            .is_some_and(|generation| generation > crate::MAX_SESSION_MCP_COUNTER)
        {
            return Err(MessageValidationError::InvalidSessionMcp);
        }
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.node_id.is_empty(), "node_id")?;
        validate_identity(self.payload.command_id.is_empty(), "command_id")?;
        self.payload.turn.validate()
    }
}

impl ValidateMessage for EndSessionMessage {
    /// Requires command identity.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.node_id.is_empty(), "node_id")?;
        validate_identity(self.payload.command_id.is_empty(), "command_id")
    }
}

impl ValidateMessage for SessionCommandAcceptedMessage {
    /// Requires the command identity the reply settles.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.command_id.is_empty(), "command_id")
    }
}

impl ValidateMessage for SessionCommandRejectedMessage {
    /// Requires the command identity the reply settles.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.payload.command_id.is_empty(), "command_id")
    }
}

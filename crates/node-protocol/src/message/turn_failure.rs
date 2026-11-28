//! The settlement of a kept-on-failure session's failed turn, and its outcome.
//!
//! A session that chose `keep_session_on_failure` gets exactly one `TurnFailureSettled` per failed
//! `TurnEnded` before it ends: `kept` when the session stays open for another turn, `session_ending`
//! when it will not. It carries no reason — that belongs to the `TurnEnded.failure` record — and,
//! like `TurnCancelSettled`, it is persisted before it is sent, replayed after takeover, and
//! acknowledged exactly.

use super::validation::{validate_execution_ids, validate_identity, validate_protocol_version};
use crate::{
    ExecutionId, MessageValidationError, OperationId, ProtocolVersion, Sequence, TurnId,
    ValidateMessage,
};
use serde::{Deserialize, Serialize};

/// Whether the session stays open after a failed turn.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnFailureOutcome {
    /// The session is kept and may run the next turn.
    Kept,
    /// The session will not be kept; terminal settlement follows.
    SessionEnding,
}

/// The single settlement of one failed turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TurnFailureSettled {
    pub turn_id: TurnId,
    pub outcome: TurnFailureOutcome,
}

/// One sequenced, non-terminal settlement of a session execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnFailureSettledMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: TurnFailureSettled,
}

impl ValidateMessage for TurnFailureSettledMessage {
    /// A settlement occupies its own reliable sequence and names the failed turn.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        validate_identity(self.sequence.value() == 0, "sequence")?;
        validate_identity(self.payload.turn_id.is_empty(), "turn_id")
    }
}

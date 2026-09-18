use crate::{CURRENT_PROTOCOL_VERSION, ExecutionId, NodeId, OperationId, ProtocolVersion};
use thiserror::Error;

/// Explains why a decoded or outbound typed message violates protocol invariants.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MessageValidationError {
    #[error("clone result must belong to its requested Node")]
    CloneTargetMismatch,
    #[error("clone commit must be a full hexadecimal Git object ID")]
    InvalidCloneCommit,
    #[error("clone branch must be a literal short branch name, not HEAD or a revision expression")]
    InvalidCloneBranch,
    #[error("unsupported protocol version {actual}; expected {expected}")]
    UnsupportedProtocolVersion { actual: u16, expected: u16 },
    #[error("protocol field {field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("hello must advertise at least one protocol version")]
    NoSupportedProtocolVersions,
    #[error("hello advertises protocol version {version} more than once")]
    DuplicateProtocolVersion { version: u16 },
    #[error("hello does not advertise envelope protocol version {version}")]
    EnvelopeVersionNotAdvertised { version: u16 },
    #[error("hello-accepted selected version {selected} differs from envelope version {envelope}")]
    SelectedVersionMismatch { selected: u16, envelope: u16 },
    #[error("hello-accepted must advertise at least one execution capability")]
    NoExecutionCapabilities,
    #[error("hello-accepted advertises a capability more than once")]
    DuplicateCapability,
    #[error("completed result Node {result} differs from reporting Node {reporter}")]
    CompletedNodeMismatch { reporter: NodeId, result: NodeId },
}

/// Centralizes wire invariants used identically for outbound and decoded messages.
pub trait ValidateMessage {
    /// Rejects values that are structurally typed but invalid for this protocol version.
    fn validate(&self) -> Result<(), MessageValidationError>;
}

/// Validates stable correlation identities shared by commands, queries, events, and results.
pub(super) fn validate_execution_ids(
    operation_id: &OperationId,
    execution_id: &ExecutionId,
) -> Result<(), MessageValidationError> {
    validate_identity(operation_id.is_empty(), "operation_id")?;
    validate_identity(execution_id.is_empty(), "execution_id")
}

/// Refuses envelopes whose encoding version this crate does not implement.
pub(super) fn validate_protocol_version(
    protocol_version: ProtocolVersion,
) -> Result<(), MessageValidationError> {
    if protocol_version == CURRENT_PROTOCOL_VERSION {
        return Ok(());
    }
    Err(MessageValidationError::UnsupportedProtocolVersion {
        actual: protocol_version.value(),
        expected: CURRENT_PROTOCOL_VERSION.value(),
    })
}

/// Converts an opaque identity's empty check into a field-addressed protocol error.
pub(super) fn validate_identity(
    is_empty: bool,
    field: &'static str,
) -> Result<(), MessageValidationError> {
    if is_empty {
        return Err(MessageValidationError::EmptyField { field });
    }
    Ok(())
}

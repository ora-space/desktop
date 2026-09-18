use super::validation::{validate_execution_ids, validate_protocol_version};
use crate::{
    CloneExecutionResult, CloneExecutionSpec, ExecutionId, MessageValidationError, OperationId,
    ProtocolVersion, RequestId, Sequence, ValidateMessage,
};
use serde::{Deserialize, Serialize};

/// Requests repository acquisition without an existing checkout or arbitrary execution options.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloneRepository {
    pub spec: CloneExecutionSpec,
}

/// One retained terminal clone event; status queries reuse its payload without acknowledging it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloneResultMessage {
    pub protocol_version: ProtocolVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: CloneExecutionResult,
}

impl ValidateMessage for CloneResultMessage {
    /// Applies clone-owned terminal checks before event delivery or replay.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        if self.request_id.as_ref().is_some_and(RequestId::is_empty) {
            return Err(MessageValidationError::EmptyField {
                field: "request_id",
            });
        }
        self.payload.validate()
    }
}

/// Correlates clone intent with the original operation and durable execution identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloneRepositoryMessage {
    pub protocol_version: ProtocolVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: CloneRepository,
}

impl ValidateMessage for CloneRepositoryMessage {
    /// Rejects invalid correlation and branch inputs before the codec writes any bytes.
    fn validate(&self) -> Result<(), MessageValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_execution_ids(&self.operation_id, &self.execution_id)?;
        if self.request_id.as_ref().is_some_and(RequestId::is_empty) {
            return Err(MessageValidationError::EmptyField {
                field: "request_id",
            });
        }
        self.payload.spec.validate()
    }
}

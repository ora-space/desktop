//! Execution status checks business-independent state and historical result ownership.

use super::validation::{
    MessageValidationError, ValidateMessage, validate_execution_ids, validate_identity,
    validate_protocol_version,
};
use crate::{
    ExecutionId, ExecutionResult, NodeId, NodeRuntimeIdentity, OperationId, ProtocolVersion,
    Sequence,
};
use serde::{Deserialize, Serialize};

/// Query for evidence retained under an existing execution identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct GetExecutionStatus {
    pub node_id: NodeId,
}

/// Acknowledgement target for an event already accepted by durable Controller storage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct EventAck {
    pub node_id: NodeId,
}

/// Evidence currently retained by a Node for one execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "result", rename_all = "snake_case")]
pub enum ExecutionState {
    Unknown,
    Accepted,
    Running,
    Completed(ExecutionResult),
}

/// Execution status associated with the Node incarnation reporting it.
/// Completed results retain their original incarnation but must belong to the reporting NodeId.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionStatus {
    pub node: NodeRuntimeIdentity,
    pub state: ExecutionState,
}

/// Complete GetExecutionStatus envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GetExecutionStatusMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: GetExecutionStatus,
}

/// Complete EventAck envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventAckMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub sequence: Sequence,
    pub payload: EventAck,
}

impl ValidateMessage for GetExecutionStatusMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            operation_id,
            execution_id,
            payload,
        } = self;

        validate_protocol_version(*protocol_version)?;
        validate_execution_ids(operation_id, execution_id)?;
        validate_identity(payload.node_id.is_empty(), "node_id")
    }
}

impl ValidateMessage for EventAckMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            operation_id,
            execution_id,
            payload,
            ..
        } = self;

        validate_protocol_version(*protocol_version)?;
        validate_execution_ids(operation_id, execution_id)?;
        validate_identity(payload.node_id.is_empty(), "node_id")
    }
}

/// Complete ExecutionStatus envelope, including only metadata valid for this message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionStatusMessage {
    pub protocol_version: ProtocolVersion,
    pub operation_id: OperationId,
    pub execution_id: ExecutionId,
    pub payload: ExecutionStatus,
}

impl ValidateMessage for ExecutionStatusMessage {
    /// Enforces this message’s semantic rules on both send and receive.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let Self {
            protocol_version,
            operation_id,
            execution_id,
            payload,
        } = self;

        validate_protocol_version(*protocol_version)?;
        validate_execution_ids(operation_id, execution_id)?;
        payload
            .node
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        if let ExecutionState::Completed(result) = &payload.state {
            result.validate()?;
            let result_node = result.node();
            // Restarted Nodes may report retained results without rewriting their origin.
            if payload.node.node_id != result_node.node_id {
                return Err(MessageValidationError::CompletedNodeMismatch {
                    reporter: payload.node.node_id.clone(),
                    result: result_node.node_id.clone(),
                });
            }
        }
        Ok(())
    }
}

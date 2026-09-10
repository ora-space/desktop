use crate::{
    CURRENT_PROTOCOL_VERSION, ControllerId, ExecutionId, NodeId, NodeRuntimeIdentity, OperationId,
    ProtocolVersion, RequestId, Sequence, WorktreeExecutionResult, WorktreeExecutionSpec,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

/// Capability names negotiated during the Controller–Node handshake.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeCapability {
    WorktreeExecution,
}

/// Controller greeting used to negotiate a protocol version for a new session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Hello {
    pub controller_id: ControllerId,
    pub supported_versions: Vec<ProtocolVersion>,
}

/// Node response binding a negotiated session to persistent and incarnation identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct HelloAccepted {
    pub selected_version: ProtocolVersion,
    pub node: NodeRuntimeIdentity,
    pub capabilities: Vec<NodeCapability>,
}

/// Liveness signal from the current Node incarnation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Heartbeat {
    pub node: NodeRuntimeIdentity,
}

/// Command asking a Node to durably ensure one task worktree exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct EnsureWorktree {
    pub spec: WorktreeExecutionSpec,
}

/// Command asking a Node to durably remove one task worktree and its owned branch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RemoveWorktree {
    pub spec: WorktreeExecutionSpec,
}

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
    Completed(WorktreeExecutionResult),
}

/// Execution status associated with the Node incarnation reporting it.
/// Completed results retain their original incarnation but must belong to the reporting NodeId.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionStatus {
    pub node: NodeRuntimeIdentity,
    pub state: ExecutionState,
}

/// Messages that a Controller may send to a Node.
///
/// Each variant carries exactly the correlation metadata valid for that message kind, so callers
/// cannot construct a Worktree execution without its stable operation and execution identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "message_type", rename_all = "snake_case")]
pub enum ControllerToNodeMessage {
    Hello {
        protocol_version: ProtocolVersion,
        payload: Hello,
    },
    EnsureWorktree {
        protocol_version: ProtocolVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<RequestId>,
        operation_id: OperationId,
        execution_id: ExecutionId,
        payload: EnsureWorktree,
    },
    RemoveWorktree {
        protocol_version: ProtocolVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<RequestId>,
        operation_id: OperationId,
        execution_id: ExecutionId,
        payload: RemoveWorktree,
    },
    GetExecutionStatus {
        protocol_version: ProtocolVersion,
        operation_id: OperationId,
        execution_id: ExecutionId,
        payload: GetExecutionStatus,
    },
    EventAck {
        protocol_version: ProtocolVersion,
        operation_id: OperationId,
        execution_id: ExecutionId,
        sequence: Sequence,
        payload: EventAck,
    },
}

/// Messages that a Node may send to a Controller.
///
/// Terminal Worktree variants always include an execution sequence, while session liveness and
/// point-in-time status replies cannot accidentally masquerade as replayable terminal events.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "message_type", rename_all = "snake_case")]
pub enum NodeToControllerMessage {
    HelloAccepted {
        protocol_version: ProtocolVersion,
        payload: HelloAccepted,
    },
    Heartbeat {
        protocol_version: ProtocolVersion,
        payload: Heartbeat,
    },
    ExecutionStatus {
        protocol_version: ProtocolVersion,
        operation_id: OperationId,
        execution_id: ExecutionId,
        payload: ExecutionStatus,
    },
    WorktreeReady {
        protocol_version: ProtocolVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<RequestId>,
        operation_id: OperationId,
        execution_id: ExecutionId,
        sequence: Sequence,
        payload: crate::WorktreeReady,
    },
    WorktreeFailed {
        protocol_version: ProtocolVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<RequestId>,
        operation_id: OperationId,
        execution_id: ExecutionId,
        sequence: Sequence,
        payload: crate::WorktreeFailed,
    },
    WorktreeRemoved {
        protocol_version: ProtocolVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<RequestId>,
        operation_id: OperationId,
        execution_id: ExecutionId,
        sequence: Sequence,
        payload: crate::WorktreeRemoved,
    },
    WorktreeRemovalFailed {
        protocol_version: ProtocolVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<RequestId>,
        operation_id: OperationId,
        execution_id: ExecutionId,
        sequence: Sequence,
        payload: crate::WorktreeRemovalFailed,
    },
}

/// Explains why a decoded or outbound typed message violates protocol invariants.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MessageValidationError {
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
    #[error("hello-accepted must advertise the worktree-execution capability")]
    WorktreeCapabilityMissing,
    #[error("hello-accepted advertises a capability more than once")]
    DuplicateCapability,
    #[error("completed result Node {result} differs from reporting Node {reporter}")]
    CompletedNodeMismatch { reporter: NodeId, result: NodeId },
}

/// Centralizes wire invariants used identically for outbound and decoded messages.
pub(crate) trait ValidateMessage {
    /// Rejects values that are structurally typed but invalid for this protocol version.
    fn validate(&self) -> Result<(), MessageValidationError>;
}

impl ValidateMessage for ControllerToNodeMessage {
    fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::Hello {
                protocol_version,
                payload,
            } => {
                validate_protocol_version(*protocol_version)?;
                validate_identity(payload.controller_id.is_empty(), "controller_id")?;
                if payload.supported_versions.is_empty() {
                    return Err(MessageValidationError::NoSupportedProtocolVersions);
                }
                let mut versions = HashSet::with_capacity(payload.supported_versions.len());
                for version in &payload.supported_versions {
                    if !versions.insert(*version) {
                        return Err(MessageValidationError::DuplicateProtocolVersion {
                            version: version.value(),
                        });
                    }
                }
                if !versions.contains(protocol_version) {
                    return Err(MessageValidationError::EnvelopeVersionNotAdvertised {
                        version: protocol_version.value(),
                    });
                }
                Ok(())
            }
            Self::EnsureWorktree {
                protocol_version,
                request_id,
                operation_id,
                execution_id,
                payload,
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    request_id.as_ref(),
                    operation_id,
                    execution_id,
                )?;
                payload
                    .spec
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
            Self::RemoveWorktree {
                protocol_version,
                request_id,
                operation_id,
                execution_id,
                payload,
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    request_id.as_ref(),
                    operation_id,
                    execution_id,
                )?;
                payload
                    .spec
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
            Self::GetExecutionStatus {
                protocol_version,
                operation_id,
                execution_id,
                payload,
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    /*request_id*/ None,
                    operation_id,
                    execution_id,
                )?;
                validate_identity(payload.node_id.is_empty(), "node_id")
            }
            Self::EventAck {
                protocol_version,
                operation_id,
                execution_id,
                payload,
                ..
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    /*request_id*/ None,
                    operation_id,
                    execution_id,
                )?;
                validate_identity(payload.node_id.is_empty(), "node_id")
            }
        }
    }
}

impl ValidateMessage for NodeToControllerMessage {
    fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::HelloAccepted {
                protocol_version,
                payload,
            } => {
                validate_protocol_version(*protocol_version)?;
                payload
                    .node
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })?;
                if payload.selected_version != *protocol_version {
                    return Err(MessageValidationError::SelectedVersionMismatch {
                        selected: payload.selected_version.value(),
                        envelope: protocol_version.value(),
                    });
                }
                let mut capabilities = HashSet::with_capacity(payload.capabilities.len());
                for capability in &payload.capabilities {
                    if !capabilities.insert(*capability) {
                        return Err(MessageValidationError::DuplicateCapability);
                    }
                }
                if !capabilities.contains(&NodeCapability::WorktreeExecution) {
                    return Err(MessageValidationError::WorktreeCapabilityMissing);
                }
                Ok(())
            }
            Self::Heartbeat {
                protocol_version,
                payload,
            } => {
                validate_protocol_version(*protocol_version)?;
                payload
                    .node
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
            Self::ExecutionStatus {
                protocol_version,
                operation_id,
                execution_id,
                payload,
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    /*request_id*/ None,
                    operation_id,
                    execution_id,
                )?;
                payload
                    .node
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })?;
                if let ExecutionState::Completed(result) = &payload.state {
                    result
                        .validate()
                        .map_err(|field| MessageValidationError::EmptyField { field })?;
                    let result_node = match result {
                        WorktreeExecutionResult::Ready(result) => &result.node,
                        WorktreeExecutionResult::Failed(result) => &result.node,
                        WorktreeExecutionResult::Removed(result) => &result.node,
                        WorktreeExecutionResult::RemovalFailed(result) => &result.node,
                    };
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
            Self::WorktreeReady {
                protocol_version,
                request_id,
                operation_id,
                execution_id,
                payload,
                ..
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    request_id.as_ref(),
                    operation_id,
                    execution_id,
                )?;
                payload
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
            Self::WorktreeFailed {
                protocol_version,
                request_id,
                operation_id,
                execution_id,
                payload,
                ..
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    request_id.as_ref(),
                    operation_id,
                    execution_id,
                )?;
                payload
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
            Self::WorktreeRemoved {
                protocol_version,
                request_id,
                operation_id,
                execution_id,
                payload,
                ..
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    request_id.as_ref(),
                    operation_id,
                    execution_id,
                )?;
                payload
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
            Self::WorktreeRemovalFailed {
                protocol_version,
                request_id,
                operation_id,
                execution_id,
                payload,
                ..
            } => {
                validate_execution_metadata(
                    *protocol_version,
                    request_id.as_ref(),
                    operation_id,
                    execution_id,
                )?;
                payload
                    .validate()
                    .map_err(|field| MessageValidationError::EmptyField { field })
            }
        }
    }
}

/// Validates stable correlation identities shared by commands, queries, events, and results.
fn validate_execution_metadata(
    protocol_version: ProtocolVersion,
    request_id: Option<&RequestId>,
    operation_id: &OperationId,
    execution_id: &ExecutionId,
) -> Result<(), MessageValidationError> {
    validate_protocol_version(protocol_version)?;
    if request_id.is_some_and(RequestId::is_empty) {
        return Err(MessageValidationError::EmptyField {
            field: "request_id",
        });
    }
    validate_identity(operation_id.is_empty(), "operation_id")?;
    validate_identity(execution_id.is_empty(), "execution_id")
}

/// Refuses envelopes whose encoding version this crate does not implement.
fn validate_protocol_version(
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
fn validate_identity(is_empty: bool, field: &'static str) -> Result<(), MessageValidationError> {
    if is_empty {
        return Err(MessageValidationError::EmptyField { field });
    }
    Ok(())
}

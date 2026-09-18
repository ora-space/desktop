//! Shared wire contract between Ora Controller and execution Nodes.
//!
//! The crate owns typed protocol messages and their binary framing. It deliberately has no
//! transport, persistence, filesystem, Git, or Ora application-domain dependencies.

mod domain;
mod frame;
mod identity;
mod message;

pub use domain::{
    BranchName, CloneExecutionResult, CloneExecutionSpec, CloneFailed, CloneFailureCode,
    CloneReady, CloneRepositoryUrl, CloneResidual, CommitId, ExecutionResult, GitRef,
    InvalidCloneRepositoryUrl, MainWorkspaceBinding, NodePath, RepositoryRef,
    WorktreeExecutionResult, WorktreeExecutionSpec, WorktreeFacts, WorktreeFailed, WorktreeFailure,
    WorktreeFailureCode, WorktreePathPolicy, WorktreeReady, WorktreeRemovalFailed,
    WorktreeRemovalOutcome, WorktreeRemoved,
};
pub use frame::{
    FrameError, MAX_FRAME_LENGTH, NODE_MESSAGE_FRAME_TYPE, read_controller_message,
    read_node_message, write_controller_message, write_node_message,
};
pub use identity::{
    CURRENT_PROTOCOL_VERSION, ControllerId, ExecutionId, NodeId, NodeIncarnationId,
    NodeRuntimeIdentity, OperationId, ProtocolVersion, RepositoryId, RequestId, Sequence,
    WorkspaceId, WorktreeId,
};
pub use message::{
    CloneRepository, CloneRepositoryMessage, CloneResultMessage, ControllerToNodeMessage,
    EnsureWorktree, EnsureWorktreeMessage, EventAck, EventAckMessage, ExecutionState,
    ExecutionStatus, ExecutionStatusMessage, GetExecutionStatus, GetExecutionStatusMessage,
    Heartbeat, HeartbeatMessage, Hello, HelloAccepted, HelloAcceptedMessage, HelloMessage,
    MessageValidationError, NodeCapability, NodeToControllerMessage, RemoveWorktree,
    RemoveWorktreeMessage, ValidateMessage, WorktreeFailedMessage, WorktreeReadyMessage,
    WorktreeRemovalFailedMessage, WorktreeRemovedMessage,
};

//! Shared wire contract between Ora Controller and execution Nodes.
//!
//! The crate owns typed protocol messages and their binary framing. It deliberately has no
//! transport, persistence, filesystem, Git, or Ora application-domain dependencies.

mod domain;
mod frame;
mod identity;
mod message;

pub use domain::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionResult, AgentSessionSpec, BranchName,
    CloneExecutionResult, CloneExecutionSpec, CloneFailed, CloneFailureCode, CloneReady,
    CloneRepositoryUrl, CloneResidual, CommitId, ContentBlock, DeliverRevisionSpec, DownloadMethod,
    ExecutionResult, GitIdentity, GitRef, InstallPluginsSpec, InvalidCloneRepositoryUrl,
    MAX_THREAD_RECORD_BYTES, MAX_USER_TURN_TEXT_BYTES, MainWorkspaceBinding, NodePath,
    ObjectDownloadGrant, ObjectKey, ObjectUploadGrant, PluginDownload, PluginExecutionResult,
    PluginFailureCode, PluginId, PluginInstall, PluginItemOutcome, PluginItemResult, PluginRelease,
    PluginRemoval, PluginTargetDownload, PluginVersion, PluginsCompleted, PluginsFailed,
    PluginsFailureCode, PresignedUrl, PriorRevision, PriorRevisionCommit, REVISION_REF_PREFIX,
    RemovePluginsSpec, RepositoryRef, RevisionDelivered, RevisionExecutionResult, RevisionFailed,
    RevisionFailureCode, RevisionRef, RevisionUnchanged, Sha256Digest, StoredObject, ThreadEvent,
    UploadMethod, UserTurn, WorktreeExecutionResult, WorktreeExecutionSpec, WorktreeFacts,
    WorktreeFailed, WorktreeFailure, WorktreeFailureCode, WorktreePathPolicy, WorktreeReady,
    WorktreeRemovalFailed, WorktreeRemovalOutcome, WorktreeRemoved,
};
pub use frame::{
    FrameError, MAX_FRAME_LENGTH, NODE_MESSAGE_FRAME_TYPE, decode_controller_frame,
    decode_node_frame, encode_controller_frame, encode_node_frame, read_controller_message,
    read_node_message, write_controller_message, write_node_message,
};
pub use identity::{
    CURRENT_PROTOCOL_VERSION, CommandId, ControllerId, ExecutionId, NodeId, NodeIncarnationId,
    NodeRuntimeIdentity, OperationId, ProtocolVersion, RepositoryId, RequestId, RevisionId,
    Sequence, TurnId, WorkspaceId, WorktreeId,
};
pub use message::{
    AgentSessionEndedMessage, CloneRepository, CloneRepositoryMessage, CloneResultMessage,
    ControlledClone, ControlledDeliverRevision, ControlledPlugins, ControlledStartAgentSession,
    ControllerHeartbeat, ControllerHeartbeatMessage, ControllerToNodeMessage, DeliverRevision,
    DeliverRevisionMessage, DownloadGrant, DownloadGrantMessage, DownloadGrantNeeded,
    DownloadGrantNeededMessage, EndSession, EndSessionMessage, EndSessionReason, EnsureWorktree,
    EnsureWorktreeMessage, EventAck, EventAckMessage, ExecutionState, ExecutionStatus,
    ExecutionStatusMessage, GetExecutionStatus, GetExecutionStatusMessage, Heartbeat,
    HeartbeatMessage, Hello, HelloAccepted, HelloAcceptedMessage, HelloMessage, InstallPlugins,
    InstallPluginsMessage, MessageValidationError, NodeCapability, NodeToControllerMessage,
    PluginCommand, PluginsResultMessage, RemovePlugins, RemovePluginsMessage, RemoveWorktree,
    RemoveWorktreeMessage, RevisionResultMessage, RuntimeBinding, RuntimeControlState,
    SessionCommandAccepted, SessionCommandAcceptedMessage, SessionCommandRejected,
    SessionCommandRejectedMessage, SessionCommandRejection, StartAgentSession,
    StartAgentSessionMessage, SubmitUserTurn, SubmitUserTurnMessage, ThreadEventMessage,
    UploadGrant, UploadGrantMessage, UploadGrantNeeded, UploadGrantNeededMessage, ValidateMessage,
    WorktreeFailedMessage, WorktreeReadyMessage, WorktreeRemovalFailedMessage,
    WorktreeRemovedMessage,
};

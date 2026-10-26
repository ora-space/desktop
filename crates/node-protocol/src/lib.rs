//! Shared wire contract between Ora Controller and execution Nodes.
//!
//! The crate owns typed protocol messages and their binary framing. It deliberately has no
//! transport, persistence, filesystem, Git, or Ora application-domain dependencies.

mod domain;
mod frame;
mod identity;
mod message;
pub use message::{ThreadPreview, ThreadPreviewBlock, ThreadPreviewMessage};

pub use domain::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionResult, AgentSessionSpec, BoundModel,
    BranchName, CloneExecutionResult, CloneExecutionSpec, CloneFailed, CloneFailureCode,
    CloneReady, CloneResidual, CommitId, ContentBlock, DELIVERY_REF_PREFIX, DeliverRevisionSpec,
    DeliverScheduledArtifactsSpec, DeliveryRef, ExecutionResult, FrozenAgentConfiguration,
    GitIdentity, GitRef, GitRepositoryUrl, InstallPluginsSpec, InvalidGitRepositoryUrl,
    InvalidGitServerRef, MAX_AGENT_SETTINGS_BYTES, MAX_SCHEDULED_ARTIFACT_BYTES,
    MAX_SCHEDULED_ARTIFACT_REJECTION_NAME_BYTES, MAX_SCHEDULED_ARTIFACT_REJECTIONS,
    MAX_SCHEDULED_ARTIFACT_TOTAL_BYTES, MAX_SCHEDULED_ARTIFACTS_PER_RUN,
    MAX_SCHEDULED_VERDICT_DETAIL_CHARS, MAX_THREAD_RECORD_BYTES, MAX_USER_TURN_TEXT_BYTES,
    MAX_VISIBLE_TEXT_BYTES, MAX_WORKSPACE_BRANCH_BYTES, MAX_WORKSPACE_DIRECTORY,
    MODEL_ADAPTER_CONTRACT_VERSION, MainWorkspaceBinding, ModelAdapterCapability, ModelProtocol,
    NodePath, ObjectKey, ObjectUploadGrant, PluginAgentConfiguration, PluginDownload,
    PluginExecutionResult, PluginFailureCode, PluginId, PluginInstall, PluginItemOutcome,
    PluginItemResult, PluginRelease, PluginRemoval, PluginTargetDownload, PluginVersion,
    PluginsCompleted, PluginsFailed, PluginsFailureCode, PresignedUrl, PriorRevision,
    PriorRevisionCommit, REVISION_REF_PREFIX, RemovePluginsSpec, RepositoryRef, RevisionDelivered,
    RevisionExecutionResult, RevisionFailed, RevisionFailureCode, RevisionRef, RevisionUnchanged,
    SCHEDULED_ARTIFACT_DIRECTORY, ScheduledArtifactItem, ScheduledArtifactKind,
    ScheduledArtifactManifest, ScheduledArtifactRejection, ScheduledArtifactRejectionReason,
    ScheduledArtifactUpload, ScheduledArtifactUploadGrant, ScheduledArtifactsDelivered,
    ScheduledArtifactsEvidence, ScheduledArtifactsExecutionResult, ScheduledArtifactsFailed,
    ScheduledArtifactsFailureReason, ScheduledRunResult, ScheduledVerdict, SessionTrigger,
    Sha256Digest, SnapshotRef, StoredObject, ThreadEvent, UploadMethod, UserTurn, WorkspaceListing,
    WorkspaceRef, WorktreeExecutionResult, WorktreeExecutionSpec, WorktreeFacts, WorktreeFailed,
    WorktreeFailure, WorktreeFailureCode, WorktreePathPolicy, WorktreeReady, WorktreeRemovalFailed,
    WorktreeRemovalOutcome, WorktreeRemoved, is_canonical_uuid, is_full_lowercase_commit,
    is_workspace_branch,
};
pub use frame::{
    FrameError, MAX_FRAME_LENGTH, NODE_MESSAGE_FRAME_TYPE, decode_controller_frame,
    decode_node_frame, encode_controller_frame, encode_node_frame, read_controller_message,
    read_node_message, write_controller_message, write_node_message,
};
pub use identity::{
    CURRENT_PROTOCOL_VERSION, CommandId, ControllerId, ExecutionId, ModelBindingId, NodeId,
    NodeIncarnationId, NodeRuntimeIdentity, OperationId, ProtocolVersion, RepositoryId, RequestId,
    RevisionId, Sequence, TurnId, WorkspaceId, WorktreeId,
};
pub use message::{
    AgentSessionEndedMessage, CloneRepository, CloneRepositoryMessage, CloneResultMessage,
    ControlledClone, ControlledDeliverRevision, ControlledDeliverScheduledArtifacts,
    ControlledPlugins, ControlledStartAgentSession, ControllerHeartbeat,
    ControllerHeartbeatMessage, ControllerToNodeMessage, DeliverRevision, DeliverRevisionMessage,
    DeliverScheduledArtifacts, DeliverScheduledArtifactsMessage, EndSession, EndSessionMessage,
    EndSessionReason, EnsureWorktree, EnsureWorktreeMessage, EventAck, EventAckMessage,
    ExecutionFence, ExecutionNonAcceptance, ExecutionOrigin, ExecutionState, ExecutionStatus,
    ExecutionStatusMessage, GetExecutionStatus, GetExecutionStatusMessage, Heartbeat,
    HeartbeatMessage, Hello, HelloAccepted, HelloAcceptedMessage, HelloMessage, InstallPlugins,
    InstallPluginsMessage, MessageValidationError, NodeCapability, NodeToControllerMessage,
    PluginCommand, PluginsResultMessage, RemovePlugins, RemovePluginsMessage, RemoveWorktree,
    RemoveWorktreeMessage, RevisionResultMessage, RuntimeBinding, RuntimeControlState,
    ScheduledArtifactGrantNeeded, ScheduledArtifactGrantNeededMessage, ScheduledArtifactGrants,
    ScheduledArtifactGrantsMessage, ScheduledArtifactGrantsRefusedMessage,
    ScheduledArtifactsResultMessage, SessionCommandAccepted, SessionCommandAcceptedMessage,
    SessionCommandRejected, SessionCommandRejectedMessage, SessionCommandRejection,
    StartAgentSession, StartAgentSessionMessage, StopContainment, StopUnconfirmed, SubmitUserTurn,
    SubmitUserTurnMessage, ThreadEventMessage, UploadGrant, UploadGrantMessage, UploadGrantNeeded,
    UploadGrantNeededMessage, ValidateMessage, WorktreeFailedMessage, WorktreeReadyMessage,
    WorktreeRemovalFailedMessage, WorktreeRemovedMessage,
};

mod agent_session;
mod session_mcp;
mod session_mcp_lifecycle;
pub use session_mcp::{
    MAX_SESSION_MCP_SERVERS, MAX_SESSION_MCP_SETTINGS_BYTES, SessionMcpBinding, SessionMcpTransport,
};
pub(crate) use session_mcp_lifecycle::validate_update_members;
pub use session_mcp_lifecycle::{
    MAX_SESSION_MCP_COUNTER, SessionMcpConvergence, SessionMcpErrorCode, SessionMcpFailure,
    SessionMcpHealth, SessionMcpMemberIdentity, SessionMcpMemberState, SessionMcpProbeErrorCode,
    SessionMcpRevisionIdentity, SessionMcpState, SessionMcpUnavailable, SessionMcpUnknownReason,
    SessionMcpUpdateMember,
};
mod execution;
mod git_server;
mod model_adapter;
mod plugin;
mod repository;
mod repository_result;
mod revision;
mod revision_restore;
mod scheduled_run;
mod worktree;

pub use agent_session::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionResult, AgentSessionSpec, ContentBlock,
    GitIdentity, MAX_THREAD_RECORD_BYTES, MAX_USER_TURN_TEXT_BYTES, MAX_VISIBLE_TEXT_BYTES,
    MAX_WORKSPACE_DIRECTORY, ThreadEvent, UserTurn, WorkspaceListing,
};
pub use execution::ExecutionResult;
pub use git_server::{
    DELIVERY_REF_PREFIX, DeliveryRef, GitRepositoryUrl, InvalidGitRepositoryUrl,
    InvalidGitServerRef, MAX_WORKSPACE_BRANCH_BYTES, SnapshotRef, WorkspaceRef, is_canonical_uuid,
    is_full_lowercase_commit, is_workspace_branch,
};
pub use model_adapter::{
    BoundModel, FrozenAgentConfiguration, MAX_AGENT_SETTINGS_BYTES, MODEL_ADAPTER_CONTRACT_VERSION,
    ModelAdapterCapability, ModelProtocol,
};
pub use plugin::{
    InstallPluginsSpec, PluginAgentConfiguration, PluginDownload, PluginExecutionResult,
    PluginFailureCode, PluginId, PluginInstall, PluginItemOutcome, PluginItemResult, PluginRelease,
    PluginRemoval, PluginTargetDownload, PluginVersion, PluginsCompleted, PluginsFailed,
    PluginsFailureCode, RemovePluginsSpec, Sha256Digest,
};
pub use repository::CloneExecutionSpec;
pub use repository_result::{
    CloneExecutionResult, CloneFailed, CloneFailureCode, CloneReady, CloneResidual,
};
pub(crate) use revision::validate_commit;
pub use revision::{
    DeliverRevisionSpec, ObjectKey, ObjectUploadGrant, PresignedUrl, REVISION_REF_PREFIX,
    RevisionDelivered, RevisionExecutionResult, RevisionFailed, RevisionFailureCode, RevisionRef,
    RevisionUnchanged, StoredObject, UploadMethod,
};
pub use revision_restore::{PriorRevision, PriorRevisionCommit};
pub use scheduled_run::{
    DeliverScheduledArtifactsSpec, MAX_SCHEDULED_ARTIFACT_BYTES,
    MAX_SCHEDULED_ARTIFACT_REJECTION_NAME_BYTES, MAX_SCHEDULED_ARTIFACT_REJECTIONS,
    MAX_SCHEDULED_ARTIFACT_TOTAL_BYTES, MAX_SCHEDULED_ARTIFACTS_PER_RUN,
    MAX_SCHEDULED_VERDICT_DETAIL_CHARS, SCHEDULED_ARTIFACT_DIRECTORY, ScheduledArtifactItem,
    ScheduledArtifactKind, ScheduledArtifactManifest, ScheduledArtifactRejection,
    ScheduledArtifactRejectionReason, ScheduledArtifactUpload, ScheduledArtifactUploadGrant,
    ScheduledArtifactsDelivered, ScheduledArtifactsEvidence, ScheduledArtifactsExecutionResult,
    ScheduledArtifactsFailed, ScheduledArtifactsFailureReason, ScheduledRunResult,
    ScheduledVerdict, SessionTrigger,
};
pub use worktree::*;

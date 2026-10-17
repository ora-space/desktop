mod agent_session;
mod execution;
mod git_server;
mod plugin;
mod repository;
mod repository_result;
mod revision;
mod revision_restore;
mod worktree;

pub use agent_session::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionResult, AgentSessionSpec, ContentBlock,
    GitIdentity, MAX_THREAD_RECORD_BYTES, MAX_USER_TURN_TEXT_BYTES, MAX_WORKSPACE_DIRECTORY,
    ThreadEvent, UserTurn, WorkspaceListing,
};
pub use execution::ExecutionResult;
pub use git_server::{
    DELIVERY_REF_PREFIX, DeliveryRef, GitRepositoryUrl, InvalidGitRepositoryUrl,
    InvalidGitServerRef, MAX_WORKSPACE_BRANCH_BYTES, SnapshotRef, WorkspaceRef, is_canonical_uuid,
    is_full_lowercase_commit, is_workspace_branch,
};
pub use plugin::{
    InstallPluginsSpec, PluginDownload, PluginExecutionResult, PluginFailureCode, PluginId,
    PluginInstall, PluginItemOutcome, PluginItemResult, PluginRelease, PluginRemoval,
    PluginTargetDownload, PluginVersion, PluginsCompleted, PluginsFailed, PluginsFailureCode,
    RemovePluginsSpec, Sha256Digest,
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
pub use worktree::*;

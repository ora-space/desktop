use crate::{NodeId, NodeIncarnationId, WorkspaceId, WorktreeId};
use serde::{Deserialize, Serialize};

/// Opaque reference used by a Node to resolve a registered repository.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RepositoryRef(String);

impl RepositoryRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the opaque wire representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Path represented in the target Node's filesystem namespace.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct NodePath(String);

impl NodePath {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the Node-scoped path representation without interpreting it locally.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Git reference resolved by the target Node against its registered repository.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct GitRef(String);

impl GitRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the reference exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Expected local branch for a task worktree.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BranchName(String);

impl BranchName {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the branch name exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Immutable Git commit observed by the target Node.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CommitId(String);

impl CommitId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the commit identifier exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Binds a registered Main Workspace to its Node-scoped checkout path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MainWorkspaceBinding {
    pub workspace_id: WorkspaceId,
    pub path: NodePath,
}

/// Lets the Node choose an authorized root while retaining a stable task directory name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorktreePathPolicy {
    NodeManaged { directory_name: String },
}

/// Complete normalized input shared by create, remove, deduplication, and reconciliation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeExecutionSpec {
    pub node_id: NodeId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub repository: RepositoryRef,
    pub main_workspace: MainWorkspaceBinding,
    pub base_ref: GitRef,
    pub expected_branch: BranchName,
    pub path_policy: WorktreePathPolicy,
}

/// Node identity attached to observations so paths cannot be reused across runtimes accidentally.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct NodeRuntimeIdentity {
    pub node_id: NodeId,
    pub incarnation_id: NodeIncarnationId,
}

/// Facts confirmed after a task worktree has been created or reconciled.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeFacts {
    pub path: NodePath,
    pub branch: BranchName,
    pub base_commit: CommitId,
}

/// Stable failure categories that a Controller can persist without parsing diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeFailureCode {
    RepositoryNotFound,
    MainWorkspaceNotFound,
    InvalidMainWorkspace,
    PathOutsideAuthorizedRoot,
    BaseRefNotFound,
    BranchConflict,
    WorktreeConflict,
    IdentityConflict,
    OperationFailed,
    ResultUnknown,
}

/// Structured execution failure with a stable category and diagnostic detail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeFailure {
    pub code: WorktreeFailureCode,
    pub message: String,
}

/// Successful result of ensuring a task worktree exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeReady {
    pub node: NodeRuntimeIdentity,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub facts: WorktreeFacts,
}

/// Failed result of ensuring a task worktree exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeFailed {
    pub node: NodeRuntimeIdentity,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub failure: WorktreeFailure,
}

/// Distinguishes a performed removal from an already-absent idempotent success.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeRemovalOutcome {
    Removed,
    AlreadyAbsent,
}

/// Successful result of removing a task worktree and its owned branch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeRemoved {
    pub node: NodeRuntimeIdentity,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub outcome: WorktreeRemovalOutcome,
}

/// Failed result of removing a task worktree.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeRemovalFailed {
    pub node: NodeRuntimeIdentity,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub failure: WorktreeFailure,
}

/// Terminal result retained by a Node for replay and execution-status queries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum WorktreeExecutionResult {
    Ready(WorktreeReady),
    Failed(WorktreeFailed),
    Removed(WorktreeRemoved),
    RemovalFailed(WorktreeRemovalFailed),
}

impl WorktreeExecutionSpec {
    /// Rejects empty opaque values before execution input crosses the protocol seam.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.node_id.is_empty() {
            return Err("node_id");
        }
        if self.workspace_id.is_empty() {
            return Err("workspace_id");
        }
        if self.worktree_id.is_empty() {
            return Err("worktree_id");
        }
        if self.repository.0.trim().is_empty() {
            return Err("repository");
        }
        if self.main_workspace.workspace_id.is_empty() {
            return Err("main_workspace.workspace_id");
        }
        if self.main_workspace.path.0.trim().is_empty() {
            return Err("main_workspace.path");
        }
        if self.base_ref.0.trim().is_empty() {
            return Err("base_ref");
        }
        if self.expected_branch.0.trim().is_empty() {
            return Err("expected_branch");
        }
        match &self.path_policy {
            WorktreePathPolicy::NodeManaged { directory_name }
                if directory_name.trim().is_empty() =>
            {
                Err("path_policy.directory_name")
            }
            WorktreePathPolicy::NodeManaged { .. } => Ok(()),
        }
    }
}

impl NodeRuntimeIdentity {
    /// Ensures observations carry both persistent and incarnation Node identities.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.node_id.is_empty() {
            return Err("node.node_id");
        }
        if self.incarnation_id.is_empty() {
            return Err("node.incarnation_id");
        }
        Ok(())
    }
}

impl WorktreeExecutionResult {
    /// Applies the invariants owned by the concrete terminal result variant.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Ready(result) => result.validate(),
            Self::Failed(result) => result.validate(),
            Self::Removed(result) => result.validate(),
            Self::RemovalFailed(result) => result.validate(),
        }
    }
}

impl WorktreeReady {
    /// Ensures a successful create result contains complete Node-scoped facts.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.node.validate()?;
        if self.facts.path.0.trim().is_empty() {
            return Err("facts.path");
        }
        if self.facts.branch.0.trim().is_empty() {
            return Err("facts.branch");
        }
        if self.facts.base_commit.0.trim().is_empty() {
            return Err("facts.base_commit");
        }
        validate_result_ids(&self.workspace_id, &self.worktree_id)
    }
}

impl WorktreeFailed {
    /// Ensures a create failure remains attributable and diagnostically useful.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.node.validate()?;
        validate_failure(&self.failure)?;
        validate_result_ids(&self.workspace_id, &self.worktree_id)
    }
}

impl WorktreeRemoved {
    /// Ensures a removal result identifies the exact managed resource.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.node.validate()?;
        validate_result_ids(&self.workspace_id, &self.worktree_id)
    }
}

impl WorktreeRemovalFailed {
    /// Ensures a removal failure remains attributable and diagnostically useful.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.node.validate()?;
        validate_failure(&self.failure)?;
        validate_result_ids(&self.workspace_id, &self.worktree_id)
    }
}

/// Rejects terminal results that cannot be related to a Workspace and worktree.
fn validate_result_ids(
    workspace_id: &WorkspaceId,
    worktree_id: &WorktreeId,
) -> Result<(), &'static str> {
    if workspace_id.is_empty() {
        return Err("workspace_id");
    }
    if worktree_id.is_empty() {
        return Err("worktree_id");
    }
    Ok(())
}

/// Keeps a stable failure code from arriving without diagnostic detail.
fn validate_failure(failure: &WorktreeFailure) -> Result<(), &'static str> {
    if failure.message.trim().is_empty() {
        return Err("failure.message");
    }
    Ok(())
}

use super::git_server::is_full_lowercase_commit;
use crate::{
    CommitId, GitRepositoryUrl, MessageValidationError, NodeId, SnapshotRef, WorkspaceRef,
};
use serde::{Deserialize, Serialize};

/// A Workspace clone from a Binding snapshot on the Git Server (Node clone ADR D1).
///
/// Cloud fixes the four Git fields before the clone step reaches a Controller, which forwards
/// them verbatim. The input carries no destination, credentials or Git flags: the Node chooses
/// the directory and authenticates with its own git-access identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloneExecutionSpec {
    pub node_id: NodeId,
    pub repository_url: GitRepositoryUrl,
    pub snapshot_ref: SnapshotRef,
    pub base_commit: CommitId,
    /// Its branch short name is the checkout's local branch.
    pub workspace_ref: WorkspaceRef,
}

impl CloneExecutionSpec {
    /// Requires a full lowercase base commit that the snapshot ref is named after, so the Node
    /// can never check out a commit other than the one Cloud registered.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.node_id.is_empty() {
            return Err(MessageValidationError::EmptyField { field: "node_id" });
        }
        if !is_full_lowercase_commit(self.base_commit.as_str()) {
            return Err(MessageValidationError::InvalidCloneCommit);
        }
        if self.snapshot_ref.commit() != self.base_commit.as_str() {
            return Err(MessageValidationError::SnapshotCommitMismatch);
        }
        Ok(())
    }
}

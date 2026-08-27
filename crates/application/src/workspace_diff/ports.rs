use crate::BoxRepositorySource;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Supplies workspace-scoped Git differences while hiding Git and filesystem implementation details.
///
/// Implementations must restrict execution to the backend-resolved worktree in each request.
pub trait WorkspaceDiffReader {
    /// Computes all workspace changes against the baseline selected by the composition root.
    fn read_workspace_diff(
        &self,
        request: ReadWorkspaceDiffRequest,
    ) -> Result<WorkspaceDiffSnapshot, WorkspaceDiffReaderError>;
}

/// Selects the Git layer represented by a workspace diff snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWorkspaceDiffScope {
    Branch,
    Unstaged,
    Staged,
    Committed,
}

/// Carries the backend-owned worktree path and the comparison baseline, when one is recorded.
///
/// `base_commit_id` is `None` for a workspace with no recorded baseline (a project's main
/// checkout, or a historical worktree whose creation commit was never captured). `Unstaged` and
/// `Staged` never read it; `Branch` and `Committed` require `Some` and the caller must reject the
/// request before it reaches the reader when the baseline is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadWorkspaceDiffRequest {
    pub worktree_path: PathBuf,
    pub base_commit_id: Option<String>,
    pub scope: ReadWorkspaceDiffScope,
}

/// Returns the Git revisions and unified patch used by frontend review components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDiffSnapshot {
    pub head_commit_id: String,
    pub patch: String,
}

/// Captures Git-backed diff failures converted into stable application errors by handlers.
#[derive(Debug, Error)]
pub enum WorkspaceDiffReaderError {
    #[error("workspace diff operation failed")]
    OperationFailed(#[source] BoxRepositorySource),
    /// Indicates that the diff exceeded the bounded response budget.
    #[error("workspace diff is too large: {byte_count} bytes exceeds {max_byte_count} bytes")]
    TooLarge {
        byte_count: usize,
        max_byte_count: usize,
    },
}

impl WorkspaceDiffReaderError {
    /// Wraps an infrastructure failure without flattening its `Error::source()` chain.
    pub fn operation_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OperationFailed(Box::new(error))
    }
}

/// Supplies Git writes for a workspace checkout, verified against a recorded branch when the
/// caller has one.
///
/// The `_changes`/`_branch` methods verify the checkout against a persisted `Worktree` row (an
/// isolated task worktree) before mutating. The `_worktree_changes`/`_worktree_branch` methods
/// skip that verification — used when no such row exists (a project's main checkout), which Ora
/// does not manage the way it manages an isolated task worktree, so there is nothing recorded to
/// verify staleness against.
pub trait WorkspaceGitWriter {
    /// Stages and commits every current worktree change after verifying its recorded branch.
    fn commit_changes(
        &self,
        request: CommitWorkspaceGitRequest,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError>;

    /// Pushes the verified workspace branch to its default remote.
    fn push_branch(
        &self,
        request: PushWorkspaceGitRequest,
    ) -> Result<WorkspaceGitPush, WorkspaceGitWriterError>;

    /// Stages and commits every current change in a workspace with no recorded branch to verify.
    fn commit_worktree_changes(
        &self,
        worktree_path: &Path,
        message: &str,
    ) -> Result<WorkspaceGitCommit, WorkspaceGitWriterError>;

    /// Pushes whatever branch is currently checked out in a workspace with no recorded branch.
    fn push_worktree_branch(
        &self,
        worktree_path: &Path,
    ) -> Result<WorkspaceGitPush, WorkspaceGitWriterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitWorkspaceGitRequest {
    pub worktree_path: PathBuf,
    pub expected_branch_name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushWorkspaceGitRequest {
    pub worktree_path: PathBuf,
    pub expected_branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGitCommit {
    pub commit_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGitPush {
    pub branch_name: String,
    pub remote_name: String,
}

#[derive(Debug, Error)]
pub enum WorkspaceGitWriterError {
    /// Indicates that a Git write could not be completed.
    #[error("workspace Git write failed")]
    OperationFailed(#[source] BoxRepositorySource),
}

impl WorkspaceGitWriterError {
    /// Wraps a Git failure without flattening its `Error::source()` chain.
    pub fn operation_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OperationFailed(Box::new(error))
    }
}

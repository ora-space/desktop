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

    /// Reads structured per-file staging status so the review panel can render stage toggles.
    fn read_workspace_status(
        &self,
        worktree_path: &Path,
    ) -> Result<WorkspaceStatusSnapshot, WorkspaceDiffReaderError>;
}

/// Selects the Git layer represented by a workspace diff snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWorkspaceDiffScope {
    Branch,
    Unstaged,
    Staged,
    Committed,
}

/// Selects the payload needed by the review surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadWorkspaceDiffSelection {
    Patch,
    Files,
    File {
        path: String,
        old_path: Option<String>,
    },
}

/// Describes a changed path and its optional source before a rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
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
    pub selection: ReadWorkspaceDiffSelection,
}

/// Returns the Git revisions and unified patch used by frontend review components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDiffSnapshot {
    pub head_commit_id: String,
    pub patch: String,
    pub files: Option<Vec<WorkspaceDiffFile>>,
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

    /// Stages the supplied repo-relative paths after verifying its recorded branch.
    ///
    /// An empty `paths` list stages every current change in the workspace, matching the
    /// review panel's "stage all" action.
    fn stage_changes(
        &self,
        request: StageWorkspaceGitRequest,
    ) -> Result<WorkspaceGitStage, WorkspaceGitWriterError>;

    /// Unstages the supplied repo-relative paths after verifying its recorded branch.
    fn unstage_changes(
        &self,
        request: UnstageWorkspaceGitRequest,
    ) -> Result<WorkspaceGitUnstage, WorkspaceGitWriterError>;

    /// Stages the supplied paths in a workspace with no recorded branch to verify.
    ///
    /// An empty `paths` list stages every current change in the workspace.
    fn stage_worktree_changes(
        &self,
        worktree_path: &Path,
        paths: Vec<String>,
    ) -> Result<WorkspaceGitStage, WorkspaceGitWriterError>;

    /// Unstages the supplied paths in a workspace with no recorded branch to verify.
    fn unstage_worktree_changes(
        &self,
        worktree_path: &Path,
        paths: Vec<String>,
    ) -> Result<WorkspaceGitUnstage, WorkspaceGitWriterError>;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageWorkspaceGitRequest {
    pub worktree_path: PathBuf,
    pub expected_branch_name: String,
    /// Repo-relative paths to stage; an empty list stages every current change.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstageWorkspaceGitRequest {
    pub worktree_path: PathBuf,
    pub expected_branch_name: String,
    /// Repo-relative paths to unstage; an empty list is a defensive no-op.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGitStage {
    pub staged_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGitUnstage {
    pub unstaged_paths: Vec<String>,
}

/// Represents one changed file's staging state in a workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStatusFile {
    pub path: String,
    pub is_staged: bool,
    pub is_untracked: bool,
}

/// Returns the structured per-file staging state of one workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStatusSnapshot {
    pub entries: Vec<WorkspaceStatusFile>,
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

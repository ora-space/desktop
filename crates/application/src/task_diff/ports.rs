use crate::BoxRepositorySource;
use ora_domain::{TaskDiffComment, TaskDiffCommentId, TaskId};
use std::path::PathBuf;
use thiserror::Error;

/// Supplies task-scoped Git differences while hiding Git and filesystem implementation details.
///
/// Implementations must restrict execution to the backend-resolved worktree in each request.
pub trait TaskDiffReader {
    /// Computes all task changes against the baseline selected by the composition root.
    fn read_task_diff(
        &self,
        request: ReadTaskDiffRequest,
    ) -> Result<TaskDiffSnapshot, TaskDiffReaderError>;
}

/// Selects the Git layer represented by a task diff snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadTaskDiffScope {
    Branch,
    Unstaged,
    Staged,
    Committed,
}

/// Carries the backend-owned worktree path and immutable comparison baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTaskDiffRequest {
    pub worktree_path: PathBuf,
    pub base_commit_id: String,
    pub scope: ReadTaskDiffScope,
}

/// Returns the Git revisions and unified patch used by frontend review components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDiffSnapshot {
    pub head_commit_id: String,
    pub patch: String,
}

/// Captures Git-backed diff failures converted into stable application errors by handlers.
#[derive(Debug, Error)]
pub enum TaskDiffReaderError {
    #[error("task diff operation failed")]
    OperationFailed(#[source] BoxRepositorySource),
    /// Indicates that the diff exceeded the bounded response budget.
    #[error("task diff is too large: {byte_count} bytes exceeds {max_byte_count} bytes")]
    TooLarge {
        byte_count: usize,
        max_byte_count: usize,
    },
}

impl TaskDiffReaderError {
    /// Wraps an infrastructure failure without flattening its `Error::source()` chain.
    pub fn operation_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OperationFailed(Box::new(error))
    }
}

/// Supplies task-scoped Git writes while keeping command execution outside handlers.
pub trait TaskGitWriter {
    /// Stages and commits every current worktree change.
    fn commit_changes(
        &self,
        request: CommitTaskGitRequest,
    ) -> Result<TaskGitCommit, TaskGitWriterError>;

    /// Pushes the verified task branch to its default remote.
    fn push_branch(&self, request: PushTaskGitRequest) -> Result<TaskGitPush, TaskGitWriterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitTaskGitRequest {
    pub worktree_path: PathBuf,
    pub expected_branch_name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTaskGitRequest {
    pub worktree_path: PathBuf,
    pub expected_branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGitCommit {
    pub commit_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGitPush {
    pub branch_name: String,
    pub remote_name: String,
}

#[derive(Debug, Error)]
pub enum TaskGitWriterError {
    /// Indicates that a Git write could not be completed.
    #[error("task Git write failed")]
    OperationFailed(#[source] BoxRepositorySource),
}

impl TaskGitWriterError {
    /// Wraps a Git failure without flattening its `Error::source()` chain.
    pub fn operation_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OperationFailed(Box::new(error))
    }
}

/// Supplies persistence operations for root diff discussions and replies.
///
/// Implementations must return only visible comments and preserve their stable creation order.
pub trait TaskDiffCommentRepository {
    /// Persists one new root discussion or reply.
    fn create_comment(
        &self,
        comment: TaskDiffComment,
    ) -> Result<TaskDiffComment, TaskDiffCommentRepositoryError>;

    /// Loads one visible comment by identifier.
    fn find_comment(
        &self,
        comment_id: &TaskDiffCommentId,
    ) -> Result<Option<TaskDiffComment>, TaskDiffCommentRepositoryError>;

    /// Lists every visible discussion message for one task.
    fn list_comments(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDiffComment>, TaskDiffCommentRepositoryError>;

    /// Persists a root discussion status replacement.
    fn update_comment(
        &self,
        comment: TaskDiffComment,
    ) -> Result<TaskDiffComment, TaskDiffCommentRepositoryError>;
}

/// Supplies identifiers for newly created diff comments and replies.
pub trait TaskDiffCommentIdGenerator {
    /// Produces a fresh comment identifier.
    fn generate_comment_id(&self) -> TaskDiffCommentId;
}

/// Captures comment persistence failures without leaking database-specific errors.
#[derive(Debug, Error)]
pub enum TaskDiffCommentRepositoryError {
    /// Indicates that comment persistence failed below the application port.
    #[error("task diff comment repository operation failed")]
    OperationFailed(#[source] BoxRepositorySource),
    /// Classifies an invalid comment row or request without exposing database details.
    #[error("invalid task diff comment: {0}")]
    Invalid(String),
    /// Classifies a comment write that conflicts with stored state.
    #[error("task diff comment conflicts with stored state: {0}")]
    Conflict(String),
}

impl TaskDiffCommentRepositoryError {
    /// Wraps a persistence failure without flattening its `Error::source()` chain.
    pub fn operation_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::OperationFailed(Box::new(error))
    }
}

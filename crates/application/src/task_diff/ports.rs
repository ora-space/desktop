use ora_domain::{TaskDiffComment, TaskDiffCommentId, TaskId};
use std::path::PathBuf;

/// Supplies task-scoped Git differences while hiding Git and filesystem implementation details.
///
/// Implementations must restrict execution to the backend-resolved worktree in each request.
pub trait TaskDiffReader {
    /// Computes all task changes against the immutable commit captured at task creation.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDiffReaderError {
    OperationFailed(String),
    TooLarge {
        byte_count: usize,
        max_byte_count: usize,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskGitWriterError {
    OperationFailed(String),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDiffCommentRepositoryError {
    OperationFailed(String),
}

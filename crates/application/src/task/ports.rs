use crate::{BoxRepositorySource, RepositoryError};
use ora_domain::{Task, TaskId};
use std::path::PathBuf;
use thiserror::Error;

/// Supplies application-owned persistence operations for task CRUD use cases.
///
/// Implementations are expected to hide storage details such as soft-delete columns
/// while preserving the transport-agnostic behavior required by the handlers.
pub trait TaskRepository {
    /// Persists a newly created task and returns the stored snapshot.
    fn create_task(&self, task: Task) -> Result<Task, RepositoryError>;

    /// Loads one visible task by identifier.
    fn find_task(&self, task_id: &TaskId) -> Result<Option<Task>, RepositoryError>;

    /// Lists every visible task in storage order.
    fn list_tasks(&self) -> Result<Vec<Task>, RepositoryError>;

    /// Persists a task replacement produced by the application layer.
    fn update_task(&self, task: Task) -> Result<Task, RepositoryError>;

    /// Marks a task deleted and returns whether a visible task was affected.
    fn soft_delete_task(&self, task_id: &TaskId, deleted_at: i64) -> Result<bool, RepositoryError>;
}

/// Supplies new task identifiers for create use cases.
pub trait TaskIdGenerator {
    /// Produces the identifier for a newly created task.
    fn generate_task_id(&self) -> TaskId;
}

/// Supplies linked-worktree lifecycle operations owned by task handlers.
///
/// Implementations are expected to provision and remove backend-managed task worktrees
/// while hiding Git and filesystem details from the application layer.
pub trait TaskWorktreeProvisioner {
    /// Verifies the configured project path belongs to a Git repository.
    fn validate_repository(&self) -> Result<(), TaskWorktreeProvisionerError>;

    /// Reports whether the repository already contains the task branch, including orphaned branches without worktree folders.
    fn task_branch_exists(&self, branch_name: &str) -> Result<bool, TaskWorktreeProvisionerError>;

    /// Creates the linked worktree requested by the task-create flow.
    fn create_task_worktree(
        &self,
        request: CreateTaskWorktreeRequest,
    ) -> Result<CreateTaskWorktreeResponse, TaskWorktreeProvisionerError>;

    /// Removes the linked worktree requested by task cleanup flows.
    fn delete_task_worktree(
        &self,
        request: DeleteTaskWorktreeRequest,
    ) -> Result<(), TaskWorktreeProvisionerError>;
}

/// Carries the derived Git branch and filesystem path for one new task worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskWorktreeRequest {
    pub branch_name: String,
    pub base_reference_name: String,
    pub worktree_path: PathBuf,
}

/// Returns the immutable commit from which the task worktree was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskWorktreeResponse {
    pub base_commit_id: String,
}

/// Describes how task-owned worktree deletion should behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskWorktreeDeletionMode {
    Force,
}

/// Carries the persisted branch identity for one task worktree cleanup action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTaskWorktreeRequest {
    pub branch_name: String,
    pub mode: TaskWorktreeDeletionMode,
}

/// Captures linked-worktree lifecycle failures that handlers convert into stable application errors.
#[derive(Debug, Error)]
pub enum TaskWorktreeProvisionerError {
    #[error("worktree mode requires a Git repository")]
    NotARepository,
    #[error("base branch not found: {branch_name}")]
    BaseBranchNotFound { branch_name: String },
    #[error("{context}")]
    OperationFailed {
        context: &'static str,
        #[source]
        source: BoxRepositorySource,
    },
}

impl TaskWorktreeProvisionerError {
    /// Preserves both the failed worktree operation and its infrastructure source chain.
    pub fn operation_failed(
        context: &'static str,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::OperationFailed {
            context,
            source: Box::new(error),
        }
    }
}

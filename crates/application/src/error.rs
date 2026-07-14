use crate::{
    ProjectRepositoryError, ProjectWorkContextRepositoryError, SessionRepositoryError,
    TaskRepositoryError, TaskWorktreeProvisionerError, WorktreeRepositoryError,
};
use thiserror::Error;

/// Enumerates application-visible failures that adapters must translate for callers.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("project not found: {project_id}")]
    ProjectNotFound { project_id: String },
    #[error("project repository operation failed: {message}")]
    ProjectRepository { message: String },
    #[error("project is already occupied: {project_id}")]
    ProjectOccupied { project_id: String },
    #[error("project work context not found for {surface}/{window_id}")]
    ProjectWorkContextNotFound { surface: String, window_id: String },
    #[error("project work context repository operation failed: {message}")]
    ProjectWorkContextRepository { message: String },
    #[error("task not found: {task_id}")]
    TaskNotFound { task_id: String },
    #[error("task repository operation failed: {message}")]
    TaskRepository { message: String },
    #[error("task worktree operation failed: {message}")]
    TaskWorktree { message: String },
    #[error("worktree not found: {worktree_id}")]
    WorktreeNotFound { worktree_id: String },
    #[error("worktree repository operation failed: {message}")]
    WorktreeRepository { message: String },
    #[error("session not found: {session_id}")]
    SessionNotFound { session_id: String },
    #[error("session repository operation failed: {message}")]
    SessionRepository { message: String },
}

impl ApplicationError {
    /// Maps infrastructure-facing repository failures into stable application errors.
    pub(crate) fn from_project_repository_error(error: ProjectRepositoryError) -> Self {
        match error {
            ProjectRepositoryError::OperationFailed(message) => Self::ProjectRepository { message },
        }
    }

    /// Maps project work context repository failures into stable application errors.
    pub(crate) fn from_project_work_context_repository_error(
        error: ProjectWorkContextRepositoryError,
    ) -> Self {
        match error {
            ProjectWorkContextRepositoryError::OperationFailed(message) => {
                Self::ProjectWorkContextRepository { message }
            }
        }
    }

    /// Maps task repository failures into stable application errors.
    pub(crate) fn from_task_repository_error(error: TaskRepositoryError) -> Self {
        match error {
            TaskRepositoryError::OperationFailed(message) => Self::TaskRepository { message },
        }
    }

    /// Maps task worktree lifecycle failures into stable application errors.
    pub(crate) fn from_task_worktree_provisioner_error(
        error: TaskWorktreeProvisionerError,
    ) -> Self {
        match error {
            TaskWorktreeProvisionerError::OperationFailed(message) => {
                Self::TaskWorktree { message }
            }
        }
    }

    /// Maps worktree repository failures into stable application errors.
    pub(crate) fn from_worktree_repository_error(error: WorktreeRepositoryError) -> Self {
        match error {
            WorktreeRepositoryError::OperationFailed(message) => {
                Self::WorktreeRepository { message }
            }
        }
    }

    /// Maps session repository failures into stable application errors.
    pub(crate) fn from_session_repository_error(error: SessionRepositoryError) -> Self {
        match error {
            SessionRepositoryError::OperationFailed(message) => Self::SessionRepository { message },
        }
    }
}

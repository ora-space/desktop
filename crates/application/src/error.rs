use crate::skill::SkillStorageError;
use crate::{
    AgentDefinitionRepositoryError, ProjectRepositoryError, ProjectWorkContextRepositoryError,
    SessionRepositoryError, SkillRepositoryError, TaskRepositoryError,
    TaskWorktreeProvisionerError, WorktreeRepositoryError,
};
use ora_domain::DomainModelError;
use thiserror::Error;

/// Enumerates application-visible failures that adapters must translate for callers.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("skill name must not be blank")]
    SkillNameBlank,
    #[error("invalid skill name: {name}")]
    SkillNameInvalid { name: String },
    #[error("skill name exceeds the single path segment limit")]
    SkillNameTooLong,
    #[error("skill description must not be blank")]
    SkillDescriptionBlank,
    #[error("skill description exceeds 4096 bytes")]
    SkillDescriptionTooLarge,
    #[error("skill name already exists: {name}")]
    SkillNameConflict { name: String },
    #[error("skill not found: {skill_id}")]
    SkillNotFound { skill_id: String },
    #[error("skill repository operation failed: {message}")]
    SkillRepository { message: String },
    #[error("skill storage is inconsistent for: {name}")]
    SkillStorageInconsistent { name: String },
    #[error("skill storage operation failed: {message}")]
    SkillStorage { message: String },
    #[error("agent definition name must not be blank")]
    AgentDefinitionNameBlank,
    #[error("agent definition not found: {agent_id}")]
    AgentDefinitionNotFound { agent_id: String },
    #[error("agent definition repository operation failed: {message}")]
    AgentDefinitionRepository { message: String },
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
    #[error("worktree mode requires a Git repository")]
    TaskWorktreeRequiresGitRepository,
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
    /// Converts skill-construction validation failures into application errors.
    pub(crate) fn from_skill_domain_error(error: DomainModelError) -> Self {
        match error {
            DomainModelError::EmptySkillName => Self::SkillNameBlank,
            DomainModelError::InvalidSkillName { name } => Self::SkillNameInvalid { name },
            DomainModelError::SkillNameTooLong => Self::SkillNameTooLong,
            DomainModelError::EmptySkillDescription => Self::SkillDescriptionBlank,
            DomainModelError::SkillDescriptionTooLarge => Self::SkillDescriptionTooLarge,
            _ => Self::SkillRepository {
                message: error.to_string(),
            },
        }
    }

    /// Converts skill storage failures into stable application errors.
    pub(crate) fn from_skill_storage_error(error: SkillStorageError) -> Self {
        match error {
            SkillStorageError::FormalDirectoryMissing { name } => {
                Self::SkillStorageInconsistent { name }
            }
            SkillStorageError::FormalDirectoryExists { name } => Self::SkillStorageInconsistent {
                name,
            },
            SkillStorageError::OperationFailed { message } => Self::SkillStorage { message },
        }
    }

    /// Converts a manifest rewrite failure into a stable storage-adjacent error.
    pub(crate) fn from_manifest_error(error: ora_skill_package::ManifestError) -> Self {
        Self::SkillStorage {
            message: format!("failed to rewrite the skill manifest: {error}"),
        }
    }

    /// Converts configurable-agent construction validation failures into application errors.
    pub(crate) fn from_agent_definition_domain_error(error: DomainModelError) -> Self {
        match error {
            DomainModelError::EmptyAgentDefinitionName => Self::AgentDefinitionNameBlank,
            _ => Self::AgentDefinitionRepository {
                message: error.to_string(),
            },
        }
    }

    /// Maps skill repository failures into stable application errors.
    pub(crate) fn from_skill_repository_error(error: SkillRepositoryError) -> Self {
        match error {
            SkillRepositoryError::OperationFailed(message) => Self::SkillRepository { message },
        }
    }

    /// Maps configurable-agent repository failures into stable application errors.
    pub(crate) fn from_agent_definition_repository_error(
        error: AgentDefinitionRepositoryError,
    ) -> Self {
        match error {
            AgentDefinitionRepositoryError::OperationFailed(message) => {
                Self::AgentDefinitionRepository { message }
            }
        }
    }
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
            TaskWorktreeProvisionerError::NotARepository => Self::TaskWorktreeRequiresGitRepository,
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

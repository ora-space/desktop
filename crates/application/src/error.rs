use crate::skill::SkillPackageStoreError;
use crate::{BoxRepositorySource, RepositoryError, TaskWorktreeProvisionerError};
use ora_domain::DomainModelError;
use thiserror::Error;

/// Enumerates application-visible failures that adapters must translate for callers.
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("skill name must not be blank")]
    SkillNameBlank,
    #[error("skill not found: {skill_id}")]
    SkillNotFound { skill_id: String },
    #[error("skill repository operation failed")]
    SkillRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("skill upload contained no files")]
    SkillUploadEmpty,
    #[error("skill upload exceeds the {max_files}-file limit")]
    SkillUploadTooManyFiles { max_files: usize },
    #[error("skill upload contains an unsafe path")]
    SkillUploadPathInvalid,
    #[error("skill upload contains a duplicate path")]
    SkillUploadPathDuplicate,
    #[error("skill upload is missing a root SKILL.md manifest")]
    SkillManifestMissing,
    #[error("skill manifest is invalid")]
    SkillManifestInvalid {
        #[source]
        source: BoxRepositorySource,
    },
    #[error("skill manifest name must not be blank")]
    SkillManifestNameBlank,
    #[error("skill manifest description must not be blank")]
    SkillManifestDescriptionBlank,
    #[error("skill manifest name is not a safe directory name")]
    SkillManifestNameInvalid,
    #[error("skill folder already exists: {name}")]
    SkillFolderConflict { name: String },
    #[error("skill package storage operation failed")]
    SkillPackageStorage {
        #[source]
        source: SkillPackageStoreError,
    },
    #[error("agent definition name must not be blank")]
    AgentDefinitionNameBlank,
    #[error("agent definition not found: {agent_id}")]
    AgentDefinitionNotFound { agent_id: String },
    #[error("agent definition repository operation failed")]
    AgentDefinitionRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("project not found: {project_id}")]
    ProjectNotFound { project_id: String },
    #[error("project repository operation failed")]
    ProjectRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("project is already occupied: {project_id}")]
    ProjectOccupied { project_id: String },
    #[error("project work context not found for {surface}/{window_id}")]
    ProjectWorkContextNotFound { surface: String, window_id: String },
    #[error("project work context repository operation failed")]
    ProjectWorkContextRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("task not found: {task_id}")]
    TaskNotFound { task_id: String },
    #[error("task repository operation failed")]
    TaskRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("worktree mode requires a Git repository")]
    TaskWorktreeRequiresGitRepository,
    #[error("failed to generate a unique task worktree id after {attempts} attempts")]
    TaskWorktreeIdExhausted { attempts: usize },
    #[error("worktree root configuration is unavailable")]
    TaskWorktreeRootUnavailable,
    #[error("{context}")]
    TaskFilesystem {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    TaskWorktreeProvisioner {
        #[from]
        source: TaskWorktreeProvisionerError,
    },
    #[error("worktree not found: {worktree_id}")]
    WorktreeNotFound { worktree_id: String },
    #[error("worktree repository operation failed")]
    WorktreeRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("session not found: {session_id}")]
    SessionNotFound { session_id: String },
    #[error("session repository operation failed")]
    SessionRepository {
        #[source]
        source: RepositoryError,
    },
}

impl ApplicationError {
    /// Converts skill-construction validation failures into application errors.
    pub(crate) fn from_skill_domain_error(error: DomainModelError) -> Self {
        match error {
            DomainModelError::EmptySkillName => Self::SkillNameBlank,
            _ => Self::SkillRepository {
                source: RepositoryError::new(error),
            },
        }
    }

    /// Converts configurable-agent construction validation failures into application errors.
    pub(crate) fn from_agent_definition_domain_error(error: DomainModelError) -> Self {
        match error {
            DomainModelError::EmptyAgentDefinitionName => Self::AgentDefinitionNameBlank,
            _ => Self::AgentDefinitionRepository {
                source: RepositoryError::new(error),
            },
        }
    }

    /// Maps skill repository failures into stable application errors.
    pub(crate) fn from_skill_repository_error(error: RepositoryError) -> Self {
        Self::SkillRepository { source: error }
    }

    /// Maps configurable-agent repository failures into stable application errors.
    pub(crate) fn from_agent_definition_repository_error(error: RepositoryError) -> Self {
        Self::AgentDefinitionRepository { source: error }
    }
    /// Maps infrastructure-facing repository failures into stable application errors.
    pub(crate) fn from_project_repository_error(error: RepositoryError) -> Self {
        Self::ProjectRepository { source: error }
    }

    /// Maps project work context repository failures into stable application errors.
    pub(crate) fn from_project_work_context_repository_error(error: RepositoryError) -> Self {
        Self::ProjectWorkContextRepository { source: error }
    }

    /// Maps task repository failures into stable application errors.
    pub(crate) fn from_task_repository_error(error: RepositoryError) -> Self {
        Self::TaskRepository { source: error }
    }

    /// Maps task worktree lifecycle failures into stable application errors.
    pub(crate) fn from_task_worktree_provisioner_error(
        error: TaskWorktreeProvisionerError,
    ) -> Self {
        match error {
            TaskWorktreeProvisionerError::NotARepository => Self::TaskWorktreeRequiresGitRepository,
            source @ TaskWorktreeProvisionerError::OperationFailed { .. } => {
                Self::TaskWorktreeProvisioner { source }
            }
        }
    }

    /// Maps worktree repository failures into stable application errors.
    pub(crate) fn from_worktree_repository_error(error: RepositoryError) -> Self {
        Self::WorktreeRepository { source: error }
    }

    /// Maps session repository failures into stable application errors.
    pub(crate) fn from_session_repository_error(error: RepositoryError) -> Self {
        Self::SessionRepository { source: error }
    }
}

#[cfg(test)]
impl PartialEq for ApplicationError {
    fn eq(&self, other: &Self) -> bool {
        use ApplicationError::*;

        match (self, other) {
            (SkillNameBlank, SkillNameBlank)
            | (SkillUploadEmpty, SkillUploadEmpty)
            | (SkillUploadPathInvalid, SkillUploadPathInvalid)
            | (SkillUploadPathDuplicate, SkillUploadPathDuplicate)
            | (SkillManifestMissing, SkillManifestMissing)
            | (SkillManifestInvalid { .. }, SkillManifestInvalid { .. })
            | (SkillManifestNameBlank, SkillManifestNameBlank)
            | (SkillManifestDescriptionBlank, SkillManifestDescriptionBlank)
            | (SkillManifestNameInvalid, SkillManifestNameInvalid)
            | (SkillPackageStorage { .. }, SkillPackageStorage { .. })
            | (AgentDefinitionNameBlank, AgentDefinitionNameBlank)
            | (TaskWorktreeRequiresGitRepository, TaskWorktreeRequiresGitRepository)
            | (SkillRepository { .. }, SkillRepository { .. })
            | (AgentDefinitionRepository { .. }, AgentDefinitionRepository { .. })
            | (ProjectRepository { .. }, ProjectRepository { .. })
            | (ProjectWorkContextRepository { .. }, ProjectWorkContextRepository { .. })
            | (TaskRepository { .. }, TaskRepository { .. })
            | (WorktreeRepository { .. }, WorktreeRepository { .. })
            | (SessionRepository { .. }, SessionRepository { .. }) => true,
            (SkillNotFound { skill_id: left }, SkillNotFound { skill_id: right }) => left == right,
            (
                SkillUploadTooManyFiles { max_files: left },
                SkillUploadTooManyFiles { max_files: right },
            ) => left == right,
            (SkillFolderConflict { name: left }, SkillFolderConflict { name: right }) => {
                left == right
            }
            (
                AgentDefinitionNotFound { agent_id: left },
                AgentDefinitionNotFound { agent_id: right },
            ) => left == right,
            (ProjectNotFound { project_id: left }, ProjectNotFound { project_id: right })
            | (ProjectOccupied { project_id: left }, ProjectOccupied { project_id: right }) => {
                left == right
            }
            (
                ProjectWorkContextNotFound {
                    surface: left_surface,
                    window_id: left_window,
                },
                ProjectWorkContextNotFound {
                    surface: right_surface,
                    window_id: right_window,
                },
            ) => left_surface == right_surface && left_window == right_window,
            (TaskNotFound { task_id: left }, TaskNotFound { task_id: right }) => left == right,
            (
                TaskWorktreeIdExhausted { attempts: left },
                TaskWorktreeIdExhausted { attempts: right },
            ) => left == right,
            (TaskWorktreeRootUnavailable, TaskWorktreeRootUnavailable) => true,
            (TaskFilesystem { .. }, TaskFilesystem { .. }) => true,
            (TaskWorktreeProvisioner { .. }, TaskWorktreeProvisioner { .. }) => true,
            (WorktreeNotFound { worktree_id: left }, WorktreeNotFound { worktree_id: right }) => {
                left == right
            }
            (SessionNotFound { session_id: left }, SessionNotFound { session_id: right }) => {
                left == right
            }
            _ => false,
        }
    }
}

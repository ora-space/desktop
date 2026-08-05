use crate::skill::SkillStorageError;
use crate::skill_import::SkillImportError;
use crate::{
    BoxRepositorySource, BranchListingError, RepositoryError, TaskDiffCommentRepositoryError,
    TaskDiffReaderError, TaskWorktreeProvisionerError,
};
use ora_domain::DomainModelError;
use thiserror::Error;

/// Enumerates application-visible failures that adapters must translate for callers.
#[derive(Debug, Error)]
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
    #[error("skill storage is inconsistent for: {name}")]
    SkillStorageInconsistent { name: String },
    #[error("skill storage operation failed")]
    SkillStorage {
        #[source]
        source: SkillStorageError,
    },
    #[error("skill import failed")]
    SkillImport(#[source] SkillImportError),
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
    #[error("project branch listing operation failed")]
    ProjectBranchListing {
        #[source]
        source: BoxRepositorySource,
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
    #[error("worktree mode requires a base branch")]
    TaskBaseBranchRequired,
    #[error("base branch not found: {branch_name}")]
    TaskBaseBranchNotFound { branch_name: String },
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
    #[error("task diff operation failed")]
    TaskDiff {
        #[source]
        source: BoxRepositorySource,
    },
    #[error("task diff commit message must not be blank")]
    TaskDiffCommitMessageBlank,
    #[error("task diff baseline is unavailable")]
    TaskDiffBaselineUnavailable,
    #[error("task diff is too large: {byte_count} bytes exceeds {max_byte_count} bytes")]
    TaskDiffTooLarge {
        byte_count: usize,
        max_byte_count: usize,
    },
    #[error("task diff changed before the comment was created")]
    TaskDiffStale,
    #[error("task diff comment not found: {comment_id}")]
    TaskDiffCommentNotFound { comment_id: String },
    #[error("invalid task diff comment: {message}")]
    TaskDiffCommentInvalid { message: String },
    #[error("task diff comment repository operation failed")]
    TaskDiffCommentRepository {
        #[source]
        source: BoxRepositorySource,
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
    #[error("workflow name must not be blank")]
    WorkflowNameBlank,
    #[error("workflow not found: {workflow_id}")]
    WorkflowNotFound { workflow_id: String },
    #[error("workflow snapshot not found: {workflow_id}/{version}")]
    WorkflowSnapshotNotFound {
        workflow_id: String,
        version: String,
    },
    #[error("workflow version already exists: {workflow_id}/{version}")]
    WorkflowVersionAlreadyExists {
        workflow_id: String,
        version: String,
    },
    #[error("workflow version is invalid")]
    WorkflowVersionInvalid,
    #[error("workflow version 'draft' is reserved")]
    WorkflowVersionReserved,
    #[error("cannot delete the draft snapshot")]
    WorkflowCannotDeleteDraft,
    #[error("cannot delete the currently active version")]
    WorkflowCannotDeleteActiveVersion,
    #[error("cannot delete a workflow with live runs")]
    WorkflowActiveRuns,
    #[error("cannot rollback to the draft snapshot")]
    WorkflowCannotRollbackToDraft,
    #[error("cannot activate the draft snapshot")]
    WorkflowCannotActivateDraft,
    #[error("cannot delete a snapshot referenced by a workflow run")]
    WorkflowSnapshotInUse,
    #[error("workflow snapshot not found by id: {snapshot_id}")]
    WorkflowSnapshotNotFoundById { snapshot_id: String },
    #[error("workflow has no published snapshot")]
    WorkflowNoPublishedSnapshot,
    #[error("cannot use the draft snapshot for a workflow run")]
    WorkflowRunCannotUseDraftSnapshot,
    #[error("workflow run not found: {run_id}")]
    WorkflowRunNotFound { run_id: String },
    #[error("workflow run is active and cannot be deleted")]
    WorkflowRunActive,
    #[error("workflow repository operation failed")]
    WorkflowRepository {
        #[source]
        source: RepositoryError,
    },
    #[error("workflow run repository operation failed")]
    WorkflowRunRepository {
        #[source]
        source: RepositoryError,
    },
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
                source: RepositoryError::new(error),
            },
        }
    }

    /// Converts formal-storage failures into the stable application contract.
    pub(crate) fn from_skill_storage_error(error: SkillStorageError) -> Self {
        match error {
            SkillStorageError::FormalDirectoryMissing { name }
            | SkillStorageError::FormalDirectoryExists { name } => {
                Self::SkillStorageInconsistent { name }
            }
            source @ SkillStorageError::OperationFailed { .. } => Self::SkillStorage { source },
        }
    }

    /// Keeps manifest rewrite failures internal while preserving the formal package invariant.
    pub(crate) fn from_manifest_error(error: ora_skill_package::ManifestError) -> Self {
        Self::SkillStorage {
            source: SkillStorageError::OperationFailed {
                message: format!("failed to rewrite the skill manifest: {error}"),
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

    /// Maps Git-facing branch listing failures into stable application errors.
    pub(crate) fn from_branch_listing_error(error: BranchListingError) -> Self {
        match error {
            BranchListingError::NotARepository => Self::TaskWorktreeRequiresGitRepository,
            BranchListingError::OperationFailed(source) => Self::ProjectBranchListing { source },
        }
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
            TaskWorktreeProvisionerError::BaseBranchNotFound { branch_name } => {
                Self::TaskBaseBranchNotFound { branch_name }
            }
            source @ TaskWorktreeProvisionerError::OperationFailed { .. } => {
                Self::TaskWorktreeProvisioner { source }
            }
        }
    }

    /// Maps task diff reader failures while preserving infrastructure diagnostics.
    pub(crate) fn from_task_diff_reader_error(error: TaskDiffReaderError) -> Self {
        match error {
            TaskDiffReaderError::OperationFailed(source) => Self::TaskDiff { source },
            TaskDiffReaderError::TooLarge {
                byte_count,
                max_byte_count,
            } => Self::TaskDiffTooLarge {
                byte_count,
                max_byte_count,
            },
        }
    }

    /// Maps task diff comment persistence failures while preserving infrastructure diagnostics.
    pub(crate) fn from_task_diff_comment_repository_error(
        error: TaskDiffCommentRepositoryError,
    ) -> Self {
        match error {
            TaskDiffCommentRepositoryError::OperationFailed(source) => {
                Self::TaskDiffCommentRepository { source }
            }
        }
    }

    /// Builds an internal task diff failure for an invariant that was violated below the handler.
    pub(crate) fn task_diff_failure(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::TaskDiff {
            source: Box::new(error),
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

    /// Converts workflow-construction validation failures into application errors.
    pub(crate) fn from_workflow_domain_error(error: DomainModelError) -> Self {
        match error {
            DomainModelError::EmptyWorkflowName => Self::WorkflowNameBlank,
            _ => Self::WorkflowRepository {
                source: RepositoryError::new(error),
            },
        }
    }

    /// Maps workflow repository failures into stable application errors.
    pub(crate) fn from_workflow_repository_error(error: RepositoryError) -> Self {
        Self::WorkflowRepository { source: error }
    }

    /// Maps workflow run repository failures into stable application errors.
    pub(crate) fn from_workflow_run_repository_error(error: RepositoryError) -> Self {
        Self::WorkflowRunRepository { source: error }
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
            | (SkillStorage { .. }, SkillStorage { .. })
            | (SkillImport(_), SkillImport(_))
            | (AgentDefinitionNameBlank, AgentDefinitionNameBlank)
            | (TaskWorktreeRequiresGitRepository, TaskWorktreeRequiresGitRepository)
            | (TaskWorktreeRootUnavailable, TaskWorktreeRootUnavailable)
            | (WorkflowNameBlank, WorkflowNameBlank)
            | (WorkflowVersionInvalid, WorkflowVersionInvalid)
            | (WorkflowVersionReserved, WorkflowVersionReserved)
            | (WorkflowCannotDeleteDraft, WorkflowCannotDeleteDraft)
            | (WorkflowCannotDeleteActiveVersion, WorkflowCannotDeleteActiveVersion)
            | (WorkflowActiveRuns, WorkflowActiveRuns)
            | (WorkflowCannotRollbackToDraft, WorkflowCannotRollbackToDraft)
            | (WorkflowCannotActivateDraft, WorkflowCannotActivateDraft)
            | (WorkflowSnapshotInUse, WorkflowSnapshotInUse)
            | (WorkflowNoPublishedSnapshot, WorkflowNoPublishedSnapshot)
            | (WorkflowRunCannotUseDraftSnapshot, WorkflowRunCannotUseDraftSnapshot)
            | (WorkflowRunActive, WorkflowRunActive)
            | (WorkflowRunRepository { .. }, WorkflowRunRepository { .. })
            | (SkillRepository { .. }, SkillRepository { .. })
            | (AgentDefinitionRepository { .. }, AgentDefinitionRepository { .. })
            | (ProjectRepository { .. }, ProjectRepository { .. })
            | (ProjectBranchListing { .. }, ProjectBranchListing { .. })
            | (ProjectWorkContextRepository { .. }, ProjectWorkContextRepository { .. })
            | (TaskRepository { .. }, TaskRepository { .. })
            | (TaskWorktreeProvisioner { .. }, TaskWorktreeProvisioner { .. })
            | (TaskDiffStale, TaskDiffStale)
            | (TaskDiffCommitMessageBlank, TaskDiffCommitMessageBlank)
            | (WorktreeRepository { .. }, WorktreeRepository { .. })
            | (SessionRepository { .. }, SessionRepository { .. })
            | (WorkflowRepository { .. }, WorkflowRepository { .. })
            | (TaskFilesystem { .. }, TaskFilesystem { .. }) => true,
            (SkillNotFound { skill_id: left }, SkillNotFound { skill_id: right }) => left == right,
            (
                SkillUploadTooManyFiles { max_files: left },
                SkillUploadTooManyFiles { max_files: right },
            ) => left == right,
            (SkillFolderConflict { name: left }, SkillFolderConflict { name: right }) => {
                left == right
            }
            (SkillNameInvalid { name: left }, SkillNameInvalid { name: right })
            | (SkillNameConflict { name: left }, SkillNameConflict { name: right }) => {
                left == right
            }
            (SkillNameTooLong, SkillNameTooLong)
            | (SkillDescriptionBlank, SkillDescriptionBlank)
            | (SkillDescriptionTooLarge, SkillDescriptionTooLarge) => true,
            (SkillStorageInconsistent { name: left }, SkillStorageInconsistent { name: right }) => {
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
            (TaskBaseBranchRequired, TaskBaseBranchRequired) => true,
            (
                TaskBaseBranchNotFound { branch_name: left },
                TaskBaseBranchNotFound { branch_name: right },
            ) => left == right,
            (TaskDiff { .. }, TaskDiff { .. })
            | (TaskDiffCommentRepository { .. }, TaskDiffCommentRepository { .. }) => true,
            (
                TaskDiffCommentInvalid { message: left },
                TaskDiffCommentInvalid { message: right },
            ) => left == right,
            (TaskDiffBaselineUnavailable, TaskDiffBaselineUnavailable) => true,
            (
                TaskDiffTooLarge {
                    byte_count: left_bytes,
                    max_byte_count: left_max,
                },
                TaskDiffTooLarge {
                    byte_count: right_bytes,
                    max_byte_count: right_max,
                },
            ) => left_bytes == right_bytes && left_max == right_max,
            (
                TaskDiffCommentNotFound { comment_id: left },
                TaskDiffCommentNotFound { comment_id: right },
            ) => left == right,
            (
                TaskWorktreeIdExhausted { attempts: left },
                TaskWorktreeIdExhausted { attempts: right },
            ) => left == right,
            (WorktreeNotFound { worktree_id: left }, WorktreeNotFound { worktree_id: right }) => {
                left == right
            }
            (SessionNotFound { session_id: left }, SessionNotFound { session_id: right }) => {
                left == right
            }
            (WorkflowNotFound { workflow_id: left }, WorkflowNotFound { workflow_id: right }) => {
                left == right
            }
            (
                WorkflowSnapshotNotFound {
                    workflow_id: left_wf,
                    version: left_v,
                },
                WorkflowSnapshotNotFound {
                    workflow_id: right_wf,
                    version: right_v,
                },
            ) => left_wf == right_wf && left_v == right_v,
            (
                WorkflowSnapshotNotFoundById { snapshot_id: left },
                WorkflowSnapshotNotFoundById { snapshot_id: right },
            ) => left == right,
            (WorkflowRunNotFound { run_id: left }, WorkflowRunNotFound { run_id: right }) => {
                left == right
            }
            (
                WorkflowVersionAlreadyExists {
                    workflow_id: left_wf,
                    version: left_v,
                },
                WorkflowVersionAlreadyExists {
                    workflow_id: right_wf,
                    version: right_v,
                },
            ) => left_wf == right_wf && left_v == right_v,
            _ => false,
        }
    }
}

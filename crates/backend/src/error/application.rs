use super::skill_import::map_skill_import_error;
use super::{BackendError, ErrorClassification};
use ora_application::{
    ApplicationError, INVALID_START_OPTION_DETAIL, MISSING_REQUIRED_START_DETAIL,
    WorkflowValidationError,
};
use ora_contracts::{
    EmptyErrorParams, PublicError, SkillFolderConflictParams, WorkflowMcpNotFoundParams,
    WorkflowRunInputInvalidParams, WorkflowSkillAmbiguousParams, WorkflowSkillNotFoundParams,
    WorkflowSnapshotIncompatibleWithResumeParams,
};
use std::sync::Arc;

impl From<ApplicationError> for BackendError {
    /// Projects the highest application semantic variant and retains the complete source chain.
    fn from(error: ApplicationError) -> Self {
        let (classification, public_error, context) = match &error {
            ApplicationError::SkillNameBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::SkillNameBlank(EmptyErrorParams {}),
                "skill name must not be blank",
            ),
            ApplicationError::SkillNameInvalid { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::SkillNameInvalid(EmptyErrorParams {}),
                "skill name is invalid",
            ),
            ApplicationError::SkillNameTooLong => (
                ErrorClassification::InvalidRequest,
                PublicError::SkillNameTooLong(EmptyErrorParams {}),
                "skill name is too long",
            ),
            ApplicationError::SkillDescriptionBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::SkillDescriptionBlank(EmptyErrorParams {}),
                "skill description is blank",
            ),
            ApplicationError::SkillDescriptionTooLarge => (
                ErrorClassification::InvalidRequest,
                PublicError::SkillDescriptionTooLarge(EmptyErrorParams {}),
                "skill description is too large",
            ),
            ApplicationError::SkillNameConflict { .. } => (
                ErrorClassification::Conflict,
                PublicError::SkillNameConflict(EmptyErrorParams {}),
                "skill name already exists",
            ),
            ApplicationError::SkillReadOnly => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "plugin-provided skills are read-only",
            ),
            ApplicationError::SkillStorageInconsistent { .. } => (
                ErrorClassification::Internal,
                PublicError::SkillStorageInconsistent(EmptyErrorParams {}),
                "skill package storage is inconsistent",
            ),
            ApplicationError::SkillImport(error) => map_skill_import_error(error),
            ApplicationError::SkillNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::SkillNotFound(EmptyErrorParams {}),
                "skill not found",
            ),
            ApplicationError::SkillManifestMissing => (
                ErrorClassification::Unprocessable,
                PublicError::SkillManifestMissing(EmptyErrorParams {}),
                "skill manifest is missing",
            ),
            ApplicationError::SkillManifestInvalid { .. } => (
                ErrorClassification::Unprocessable,
                PublicError::SkillManifestInvalid(EmptyErrorParams {}),
                "skill manifest is invalid",
            ),
            ApplicationError::SkillManifestNameBlank => (
                ErrorClassification::Unprocessable,
                PublicError::SkillManifestNameBlank(EmptyErrorParams {}),
                "skill manifest name is blank",
            ),
            ApplicationError::SkillManifestDescriptionBlank => (
                ErrorClassification::Unprocessable,
                PublicError::SkillManifestDescriptionBlank(EmptyErrorParams {}),
                "skill manifest description is blank",
            ),
            ApplicationError::SkillManifestNameInvalid => (
                ErrorClassification::Unprocessable,
                PublicError::SkillManifestNameInvalid(EmptyErrorParams {}),
                "skill manifest name is invalid",
            ),
            ApplicationError::SkillFolderConflict { name } => (
                ErrorClassification::Conflict,
                PublicError::SkillFolderConflict(SkillFolderConflictParams { name: name.clone() }),
                "skill folder already exists",
            ),
            ApplicationError::AgentDefinitionNameBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::AgentNameBlank(EmptyErrorParams {}),
                "agent definition name must not be blank",
            ),
            ApplicationError::AgentDefinitionNameConflict { .. } => (
                ErrorClassification::Conflict,
                PublicError::AgentNameConflict(EmptyErrorParams {}),
                "agent definition name already exists",
            ),
            ApplicationError::AgentImportInvalid => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "agent import Markdown is invalid",
            ),
            ApplicationError::AgentImportDecisionMissing => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "agent import conflict decision is missing",
            ),
            ApplicationError::AgentDefinitionNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::AgentNotFound(EmptyErrorParams {}),
                "agent definition not found",
            ),
            ApplicationError::ProjectNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::ProjectNotFound(EmptyErrorParams {}),
                "project not found",
            ),
            ApplicationError::ProjectBranchListing { .. } => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "project branch listing operation failed",
            ),
            ApplicationError::TaskNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::TaskNotFound(EmptyErrorParams {}),
                "task not found",
            ),
            ApplicationError::TaskWorktreeRequiresGitRepository => (
                ErrorClassification::InvalidRequest,
                PublicError::WorktreeRequiresGitRepository(EmptyErrorParams {}),
                "worktree mode requires a Git repository",
            ),
            ApplicationError::TaskBaseBranchRequired => (
                ErrorClassification::InvalidRequest,
                PublicError::TaskBaseBranchRequired(EmptyErrorParams {}),
                "worktree mode requires a base branch",
            ),
            ApplicationError::TaskBaseBranchNotFound { branch_name } => (
                ErrorClassification::InvalidRequest,
                PublicError::TaskBaseBranchNotFound(ora_contracts::TaskBaseBranchNotFoundParams {
                    branch_name: branch_name.clone(),
                }),
                "base branch was not found",
            ),
            ApplicationError::WorktreeNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorktreeNotFound(EmptyErrorParams {}),
                "worktree not found",
            ),
            ApplicationError::WorkspaceDiffCommitMessageBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkspaceDiffCommitMessageBlank(EmptyErrorParams {}),
                "workspace diff commit message must not be blank",
            ),
            ApplicationError::SessionNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::SessionNotFound(EmptyErrorParams {}),
                "session not found",
            ),
            ApplicationError::SessionTitleBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "session title must not be blank",
            ),
            ApplicationError::SessionTitleTooLong => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "session title exceeds the maximum length",
            ),
            ApplicationError::ScheduledTaskNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::ScheduledTaskNotFound(EmptyErrorParams {}),
                "scheduled task not found",
            ),
            ApplicationError::ScheduledTaskConflict { .. } => (
                ErrorClassification::Conflict,
                PublicError::ScheduledTaskConflict(EmptyErrorParams {}),
                "scheduled task was modified concurrently",
            ),
            ApplicationError::ScheduledRunNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::ScheduledRunNotFound(EmptyErrorParams {}),
                "scheduled run not found",
            ),
            ApplicationError::ScheduledRunArtifactNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::ScheduledRunArtifactNotFound(EmptyErrorParams {}),
                "scheduled run artifact not found",
            ),
            ApplicationError::ScheduledRunArtifactStorage { .. }
            | ApplicationError::ScheduledRunArtifactRepository { .. } => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "scheduled run artifact operation failed",
            ),
            ApplicationError::ScheduledTaskNameBlank
            | ApplicationError::ScheduledTaskPromptBlank
            | ApplicationError::ScheduledTaskAgentRefInvalid
            | ApplicationError::ScheduledTaskIntervalTooShort
            | ApplicationError::ScheduledTaskTimeInvalid
            | ApplicationError::ScheduledTaskWeekdaysEmpty
            | ApplicationError::ScheduledTaskRequiresGitProject
            | ApplicationError::ScheduledTaskCloudUnavailable
            | ApplicationError::ScheduledTaskApprovalRequired => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "scheduled task request is invalid",
            ),
            ApplicationError::SkillRepository { .. }
            | ApplicationError::SkillStorage { .. }
            | ApplicationError::AgentDefinitionRepository { .. }
            | ApplicationError::ProjectRepository { .. }
            | ApplicationError::TaskRepository { .. }
            | ApplicationError::TaskWorkspaceIdExhausted { .. }
            | ApplicationError::TaskWorktreeRootUnavailable
            | ApplicationError::TaskFilesystem { .. }
            | ApplicationError::TaskWorktreeProvisioner { .. }
            // The run was settled or rebound concurrently while its worktree task
            // provisioned; the lease keeps the Git resources, so this is an
            // internal race outcome, not a client error.
            | ApplicationError::ScheduledRunNotAdmissible
            | ApplicationError::WorkspaceDiff { .. }
            | ApplicationError::WorkspaceRepository { .. }
            | ApplicationError::WorktreeRepository { .. }
            | ApplicationError::SessionRepository { .. }
            | ApplicationError::UserConfigRepository { .. }
            | ApplicationError::ScheduledTaskRepository { .. }
            | ApplicationError::WorkflowRepository { .. }
            | ApplicationError::WorkflowRunRepository { .. } => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "application operation failed",
            ),
            ApplicationError::WorkflowNameBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowNameBlank(EmptyErrorParams {}),
                "workflow name must not be blank",
            ),
            ApplicationError::WorkflowNameConflict { .. } => (
                ErrorClassification::Conflict,
                PublicError::WorkflowNameConflict(EmptyErrorParams {}),
                "workflow name already exists",
            ),
            ApplicationError::WorkflowNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorkflowNotFound(EmptyErrorParams {}),
                "workflow not found",
            ),
            ApplicationError::WorkflowSnapshotNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorkflowSnapshotNotFound(EmptyErrorParams {}),
                "workflow snapshot not found",
            ),
            ApplicationError::WorkflowVersionAlreadyExists { .. } => (
                ErrorClassification::Conflict,
                PublicError::WorkflowVersionAlreadyExists(EmptyErrorParams {}),
                "workflow version already exists",
            ),
            ApplicationError::WorkflowVersionInvalid => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowVersionInvalid(EmptyErrorParams {}),
                "workflow version is invalid",
            ),
            ApplicationError::WorkflowVersionReserved => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowVersionReserved(EmptyErrorParams {}),
                "workflow version 'draft' is reserved",
            ),
            ApplicationError::WorkflowCannotDeleteDraft => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotDeleteDraft(EmptyErrorParams {}),
                "cannot delete the draft workflow snapshot",
            ),
            ApplicationError::WorkflowCannotDeleteActiveVersion => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotDeleteActiveVersion(EmptyErrorParams {}),
                "cannot delete the active workflow version",
            ),
            ApplicationError::WorkflowActiveRuns => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowActiveRuns(EmptyErrorParams {}),
                "cannot delete a workflow with live runs",
            ),
            ApplicationError::WorkflowCannotRollbackToDraft => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotRollbackToDraft(EmptyErrorParams {}),
                "cannot roll back to the draft workflow snapshot",
            ),
            ApplicationError::WorkflowCannotActivateDraft => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotActivateDraft(EmptyErrorParams {}),
                "cannot activate the draft workflow snapshot",
            ),
            ApplicationError::WorkflowSnapshotInUse => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowSnapshotInUse(EmptyErrorParams {}),
                "cannot delete a snapshot referenced by a workflow run",
            ),
            ApplicationError::WorkflowSnapshotNotFoundById { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorkflowSnapshotNotFound(EmptyErrorParams {}),
                "workflow snapshot not found by id",
            ),
            ApplicationError::WorkflowNoPublishedSnapshot => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowNoPublishedSnapshot(EmptyErrorParams {}),
                "workflow has no published snapshot",
            ),
            ApplicationError::WorkflowRunCannotUseDraftSnapshot => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunCannotUseDraftSnapshot(EmptyErrorParams {}),
                "cannot use the draft snapshot for a workflow run",
            ),
            ApplicationError::WorkflowRunNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorkflowRunNotFound(EmptyErrorParams {}),
                "workflow run not found",
            ),
            ApplicationError::WorkflowRunActive => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunActive(EmptyErrorParams {}),
                "workflow run is active and cannot be deleted",
            ),
            ApplicationError::WorkflowNodeNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorkflowNodeNotFound(EmptyErrorParams {}),
                "workflow node not found",
            ),
            ApplicationError::WorkflowNodeNotAwaitingInput { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {}),
                "workflow node is not awaiting input and cannot be completed",
            ),
            ApplicationError::WorkflowNodeNotDiagnosable => (
                ErrorClassification::Conflict,
                PublicError::WorkflowNodeNotDiagnosable(EmptyErrorParams {}),
                "AI analysis is only available for a failed agent node",
            ),
            ApplicationError::WorkflowRunGraphParse(_) => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunGraphParse(EmptyErrorParams {}),
                "workflow graph is invalid",
            ),
            ApplicationError::WorkflowRunValidation(
                WorkflowValidationError::MissingRequiredStartVariable { name },
            ) => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                    variable: name.clone(),
                    reason: MISSING_REQUIRED_START_DETAIL.to_string(),
                }),
                "workflow run input value was rejected",
            ),
            ApplicationError::WorkflowRunValidation(
                WorkflowValidationError::InvalidStartVariableOption { name },
            ) => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                    variable: name.clone(),
                    reason: INVALID_START_OPTION_DETAIL.to_string(),
                }),
                "workflow run input value was rejected",
            ),
            ApplicationError::WorkflowRunValidation(_) => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunValidation(EmptyErrorParams {}),
                "workflow run is not executable",
            ),
            ApplicationError::WorkflowRunInputInvalid { variable, reason } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                    variable: variable.clone(),
                    reason: reason.clone(),
                }),
                "workflow run input value was rejected",
            ),
            ApplicationError::WorkflowSkillNotFound {
                node_id,
                node_title,
                skill_id,
            } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowSkillNotFound(Box::new(WorkflowSkillNotFoundParams {
                    node_id: node_id.clone(),
                    node_title: node_title.clone(),
                    skill_id: skill_id.clone(),
                })),
                "workflow skill not found",
            ),
            ApplicationError::WorkflowSkillAmbiguous {
                node_id,
                node_title,
                skill_ref,
                candidates,
            } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowSkillAmbiguous(Box::new(WorkflowSkillAmbiguousParams {
                    node_id: node_id.clone(),
                    node_title: node_title.clone(),
                    skill_ref: skill_ref.clone(),
                    candidates: candidates.clone(),
                })),
                "workflow skill reference is ambiguous",
            ),
            ApplicationError::WorkflowMcpNotFound {
                node_id,
                node_title,
                mcp_id,
            } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowMcpNotFound(Box::new(WorkflowMcpNotFoundParams {
                    node_id: node_id.clone(),
                    node_title: node_title.clone(),
                    mcp_id: mcp_id.clone(),
                })),
                "workflow MCP not found",
            ),
            ApplicationError::WorkflowRoleNotFound { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRoleNotFound(EmptyErrorParams {}),
                "workflow role not found",
            ),
            ApplicationError::WorkflowRunStartFailed { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunStartFailed(EmptyErrorParams {}),
                "workflow run start failed",
            ),
            ApplicationError::WorkflowRunNotRestartable => (
                ErrorClassification::Conflict,
                PublicError::WorkflowRunNotRestartable(EmptyErrorParams {}),
                "workflow run cannot be restarted while running",
            ),
            ApplicationError::WorkflowRunNotResumable => (
                ErrorClassification::Conflict,
                PublicError::WorkflowRunNotResumable(EmptyErrorParams {}),
                "workflow run cannot be resumed from failure",
            ),
            ApplicationError::WorkflowSnapshotIncompatibleWithResume { reason } => (
                ErrorClassification::Conflict,
                PublicError::WorkflowSnapshotIncompatibleWithResume(
                    WorkflowSnapshotIncompatibleWithResumeParams {
                        reason: reason.clone(),
                    },
                ),
                "workflow snapshot is incompatible with resume",
            ),
            ApplicationError::WorkflowRunNotEditable => (
                ErrorClassification::Conflict,
                PublicError::WorkflowRunNotEditable(EmptyErrorParams {}),
                "workflow run input can only be changed while the run is pending",
            ),
        };

        Self {
            classification,
            public_error,
            context: context.to_string(),
            source: Some(Arc::new(error)),
        }
    }
}

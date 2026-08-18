use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::{Config, ExportError, TS};
use uuid::Uuid;

/// Identifies one Ora request across adapters, spans, responses, and completion events.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, TS)]
#[serde(transparent)]
#[ts(type = "string", export_to = "error.ts")]
pub struct RequestId(Uuid);

impl RequestId {
    /// Generates a random version-four request identifier.
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wraps a UUID supplied by a deterministic test generator.
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the UUID representation used by protocol adapters.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for RequestId {
    /// Formats the canonical hyphenated UUID representation.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Represents the deliberately empty parameters used by errors with no safe interpolation data.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[ts(type = "{ [key: string]: never }")]
#[ts(export_to = "error.ts")]
pub struct EmptyErrorParams {}

/// Lists the finite Desktop targets that can fail to open.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "error.ts")]
pub enum OpenLocationTarget {
    Explorer,
    Terminal,
    Vscode,
}

/// Carries the safe target name for an open-location failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "error.ts")]
pub struct OpenLocationFailedParams {
    pub target: OpenLocationTarget,
}

/// Carries a validated skill name when its destination folder already exists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "error.ts")]
pub struct SkillFolderConflictParams {
    pub name: String,
}

/// Carries the user-selected base branch name when Git cannot resolve it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "error.ts")]
pub struct TaskBaseBranchNotFoundParams {
    pub branch_name: String,
}

/// Enumerates every user-visible Ora failure and its exact interpolation parameters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "code", content = "params", rename_all = "snake_case")]
#[ts(export_to = "error.ts")]
pub enum PublicError {
    InternalError(EmptyErrorParams),
    InvalidRequest(EmptyErrorParams),
    SkillNameBlank(EmptyErrorParams),
    SkillNameInvalid(EmptyErrorParams),
    SkillNameTooLong(EmptyErrorParams),
    SkillDescriptionBlank(EmptyErrorParams),
    SkillDescriptionTooLarge(EmptyErrorParams),
    SkillNameConflict(EmptyErrorParams),
    SkillNotFound(EmptyErrorParams),
    AgentNameBlank(EmptyErrorParams),
    AgentNameConflict(EmptyErrorParams),
    AgentNotFound(EmptyErrorParams),
    PluginNotFound(EmptyErrorParams),
    PluginDisabled(EmptyErrorParams),
    ProjectNotFound(EmptyErrorParams),
    TaskNotFound(EmptyErrorParams),
    ResourceInUse(EmptyErrorParams),
    WorktreeRequiresGitRepository(EmptyErrorParams),
    TaskBaseBranchRequired(EmptyErrorParams),
    TaskBaseBranchNotFound(TaskBaseBranchNotFoundParams),
    WorktreeNotFound(EmptyErrorParams),
    TaskDiffBaselineUnavailable(EmptyErrorParams),
    TaskDiffCommitMessageBlank(EmptyErrorParams),
    TaskDiffTooLarge(EmptyErrorParams),
    TaskDiffStale(EmptyErrorParams),
    TaskDiffCommentNotFound(EmptyErrorParams),
    TaskDiffCommentInvalid(EmptyErrorParams),
    TaskDiffCommentConflict(EmptyErrorParams),
    SessionNotFound(EmptyErrorParams),
    AgentCliNotFound(EmptyErrorParams),
    AgentRuntimeUnavailable(EmptyErrorParams),
    SessionBusy(EmptyErrorParams),
    SessionStopped(EmptyErrorParams),
    SessionLoadUnsupported(EmptyErrorParams),
    SessionHistoryDegraded(EmptyErrorParams),
    SessionAgentUnchanged(EmptyErrorParams),
    PermissionRequestNotPending(EmptyErrorParams),
    PermissionOptionInvalid(EmptyErrorParams),
    PromptEmpty(EmptyErrorParams),
    PromptTooLarge(EmptyErrorParams),
    TaskWorktreeUnavailable(EmptyErrorParams),
    TaskProjectRootUnavailable(EmptyErrorParams),
    FileSystemPathNotFound(EmptyErrorParams),
    SpecDocumentNotFound(EmptyErrorParams),
    WorktreeRootNotAbsolute(EmptyErrorParams),
    WorktreeRootNotDirectory(EmptyErrorParams),
    OpenLocationFailed(OpenLocationFailedParams),
    SkillManifestMissing(EmptyErrorParams),
    SkillManifestInvalid(EmptyErrorParams),
    SkillManifestNameBlank(EmptyErrorParams),
    SkillManifestDescriptionBlank(EmptyErrorParams),
    SkillManifestNameInvalid(EmptyErrorParams),
    SkillFolderConflict(SkillFolderConflictParams),
    SkillManifestNotFound(EmptyErrorParams),
    SkillManifestTooLarge(EmptyErrorParams),
    TooManySkills(EmptyErrorParams),
    ArchiveFormatUnsupported(EmptyErrorParams),
    ArchiveFormatMismatch(EmptyErrorParams),
    ArchiveCorrupt(EmptyErrorParams),
    ArchiveEncryptedUnsupported(EmptyErrorParams),
    ArchiveSpecialEntryUnsupported(EmptyErrorParams),
    ArchivePathEncodingInvalid(EmptyErrorParams),
    ArchivePathCaseConflict(EmptyErrorParams),
    PathSegmentTooLong(EmptyErrorParams),
    PathTooLong(EmptyErrorParams),
    PathTooDeep(EmptyErrorParams),
    ArchiveExpansionRatioExceeded(EmptyErrorParams),
    ImportPreparationTimeout(EmptyErrorParams),
    ImportSessionExpired(EmptyErrorParams),
    ImportSessionCancelled(EmptyErrorParams),
    ImportSessionCommitInProgress(EmptyErrorParams),
    ImportSessionAlreadyCommitted(EmptyErrorParams),
    SkillStorageInconsistent(EmptyErrorParams),
    WorkflowNameBlank(EmptyErrorParams),
    WorkflowNameConflict(EmptyErrorParams),
    WorkflowNotFound(EmptyErrorParams),
    WorkflowSnapshotNotFound(EmptyErrorParams),
    WorkflowVersionAlreadyExists(EmptyErrorParams),
    WorkflowVersionInvalid(EmptyErrorParams),
    WorkflowVersionReserved(EmptyErrorParams),
    WorkflowCannotDeleteDraft(EmptyErrorParams),
    WorkflowCannotDeleteActiveVersion(EmptyErrorParams),
    WorkflowActiveRuns(EmptyErrorParams),
    WorkflowCannotRollbackToDraft(EmptyErrorParams),
    WorkflowCannotActivateDraft(EmptyErrorParams),
    WorkflowSnapshotInUse(EmptyErrorParams),
    WorkflowNoPublishedSnapshot(EmptyErrorParams),
    WorkflowRunCannotUseDraftSnapshot(EmptyErrorParams),
    WorkflowRunNotFound(EmptyErrorParams),
    WorkflowRunActive(EmptyErrorParams),
    WorkflowRunGraphParse(EmptyErrorParams),
    WorkflowRunValidation(EmptyErrorParams),
    WorkflowSkillNotFound(EmptyErrorParams),
    WorkflowRoleNotFound(EmptyErrorParams),
    WorkflowRunStartFailed(EmptyErrorParams),
    WorkflowRunNotRestartable(EmptyErrorParams),
    WorkflowRunNotEditable(EmptyErrorParams),
}

impl PublicError {
    /// Returns the stable code without requiring adapters to repeat a second mapping table.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InternalError(_) => "internal_error",
            Self::InvalidRequest(_) => "invalid_request",
            Self::SkillNameBlank(_) => "skill_name_blank",
            Self::SkillNameInvalid(_) => "skill_name_invalid",
            Self::SkillNameTooLong(_) => "skill_name_too_long",
            Self::SkillDescriptionBlank(_) => "skill_description_blank",
            Self::SkillDescriptionTooLarge(_) => "skill_description_too_large",
            Self::SkillNameConflict(_) => "skill_name_conflict",
            Self::SkillNotFound(_) => "skill_not_found",
            Self::AgentNameBlank(_) => "agent_name_blank",
            Self::AgentNameConflict(_) => "agent_name_conflict",
            Self::AgentNotFound(_) => "agent_not_found",
            Self::PluginNotFound(_) => "plugin_not_found",
            Self::PluginDisabled(_) => "plugin_disabled",
            Self::ProjectNotFound(_) => "project_not_found",
            Self::TaskNotFound(_) => "task_not_found",
            Self::ResourceInUse(_) => "resource_in_use",
            Self::WorktreeRequiresGitRepository(_) => "worktree_requires_git_repository",
            Self::TaskBaseBranchRequired(_) => "task_base_branch_required",
            Self::TaskBaseBranchNotFound(_) => "task_base_branch_not_found",
            Self::WorktreeNotFound(_) => "worktree_not_found",
            Self::TaskDiffBaselineUnavailable(_) => "task_diff_baseline_unavailable",
            Self::TaskDiffCommitMessageBlank(_) => "task_diff_commit_message_blank",
            Self::TaskDiffTooLarge(_) => "task_diff_too_large",
            Self::TaskDiffStale(_) => "task_diff_stale",
            Self::TaskDiffCommentNotFound(_) => "task_diff_comment_not_found",
            Self::TaskDiffCommentInvalid(_) => "task_diff_comment_invalid",
            Self::TaskDiffCommentConflict(_) => "task_diff_comment_conflict",
            Self::SessionNotFound(_) => "session_not_found",
            Self::AgentCliNotFound(_) => "agent_cli_not_found",
            Self::AgentRuntimeUnavailable(_) => "agent_runtime_unavailable",
            Self::SessionBusy(_) => "session_busy",
            Self::SessionStopped(_) => "session_stopped",
            Self::SessionLoadUnsupported(_) => "session_load_unsupported",
            Self::SessionHistoryDegraded(_) => "session_history_degraded",
            Self::SessionAgentUnchanged(_) => "session_agent_unchanged",
            Self::PermissionRequestNotPending(_) => "permission_request_not_pending",
            Self::PermissionOptionInvalid(_) => "permission_option_invalid",
            Self::PromptEmpty(_) => "prompt_empty",
            Self::PromptTooLarge(_) => "prompt_too_large",
            Self::TaskWorktreeUnavailable(_) => "task_worktree_unavailable",
            Self::TaskProjectRootUnavailable(_) => "task_project_root_unavailable",
            Self::FileSystemPathNotFound(_) => "file_system_path_not_found",
            Self::SpecDocumentNotFound(_) => "spec_document_not_found",
            Self::WorktreeRootNotAbsolute(_) => "worktree_root_not_absolute",
            Self::WorktreeRootNotDirectory(_) => "worktree_root_not_directory",
            Self::OpenLocationFailed(_) => "open_location_failed",
            Self::SkillManifestMissing(_) => "skill_manifest_missing",
            Self::SkillManifestInvalid(_) => "skill_manifest_invalid",
            Self::SkillManifestNameBlank(_) => "skill_manifest_name_blank",
            Self::SkillManifestDescriptionBlank(_) => "skill_manifest_description_blank",
            Self::SkillManifestNameInvalid(_) => "skill_manifest_name_invalid",
            Self::SkillFolderConflict(_) => "skill_folder_conflict",
            Self::SkillManifestNotFound(_) => "skill_manifest_not_found",
            Self::SkillManifestTooLarge(_) => "skill_manifest_too_large",
            Self::TooManySkills(_) => "too_many_skills",
            Self::ArchiveFormatUnsupported(_) => "archive_format_unsupported",
            Self::ArchiveFormatMismatch(_) => "archive_format_mismatch",
            Self::ArchiveCorrupt(_) => "archive_corrupt",
            Self::ArchiveEncryptedUnsupported(_) => "archive_encrypted_unsupported",
            Self::ArchiveSpecialEntryUnsupported(_) => "archive_special_entry_unsupported",
            Self::ArchivePathEncodingInvalid(_) => "archive_path_encoding_invalid",
            Self::ArchivePathCaseConflict(_) => "archive_path_case_conflict",
            Self::PathSegmentTooLong(_) => "path_segment_too_long",
            Self::PathTooLong(_) => "path_too_long",
            Self::PathTooDeep(_) => "path_too_deep",
            Self::ArchiveExpansionRatioExceeded(_) => "archive_expansion_ratio_exceeded",
            Self::ImportPreparationTimeout(_) => "import_preparation_timeout",
            Self::ImportSessionExpired(_) => "import_session_expired",
            Self::ImportSessionCancelled(_) => "import_session_cancelled",
            Self::ImportSessionCommitInProgress(_) => "import_session_commit_in_progress",
            Self::ImportSessionAlreadyCommitted(_) => "import_session_already_committed",
            Self::SkillStorageInconsistent(_) => "skill_storage_inconsistent",
            Self::WorkflowNameBlank(_) => "workflow_name_blank",
            Self::WorkflowNameConflict(_) => "workflow_name_conflict",
            Self::WorkflowNotFound(_) => "workflow_not_found",
            Self::WorkflowSnapshotNotFound(_) => "workflow_snapshot_not_found",
            Self::WorkflowVersionAlreadyExists(_) => "workflow_version_already_exists",
            Self::WorkflowVersionInvalid(_) => "workflow_version_invalid",
            Self::WorkflowVersionReserved(_) => "workflow_version_reserved",
            Self::WorkflowCannotDeleteDraft(_) => "workflow_cannot_delete_draft",
            Self::WorkflowCannotDeleteActiveVersion(_) => "workflow_cannot_delete_active_version",
            Self::WorkflowActiveRuns(_) => "workflow_active_runs",
            Self::WorkflowCannotRollbackToDraft(_) => "workflow_cannot_rollback_to_draft",
            Self::WorkflowCannotActivateDraft(_) => "workflow_cannot_activate_draft",
            Self::WorkflowSnapshotInUse(_) => "workflow_snapshot_in_use",
            Self::WorkflowNoPublishedSnapshot(_) => "workflow_no_published_snapshot",
            Self::WorkflowRunCannotUseDraftSnapshot(_) => "workflow_run_cannot_use_draft_snapshot",
            Self::WorkflowRunNotFound(_) => "workflow_run_not_found",
            Self::WorkflowRunActive(_) => "workflow_run_active",
            Self::WorkflowRunGraphParse(_) => "workflow_run_graph_parse",
            Self::WorkflowRunValidation(_) => "workflow_run_validation",
            Self::WorkflowSkillNotFound(_) => "workflow_skill_not_found",
            Self::WorkflowRoleNotFound(_) => "workflow_role_not_found",
            Self::WorkflowRunStartFailed(_) => "workflow_run_start_failed",
            Self::WorkflowRunNotRestartable(_) => "workflow_run_not_restartable",
            Self::WorkflowRunNotEditable(_) => "workflow_run_not_editable",
        }
    }
}

/// Carries one validated public error and the request identifier used in runtime logs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "error.ts")]
pub struct ContractError {
    #[serde(flatten)]
    #[ts(flatten)]
    pub error: PublicError,
    pub request_id: RequestId,
}

pub(crate) fn export(config: &Config) -> Result<(), ExportError> {
    RequestId::export_all(config)?;
    EmptyErrorParams::export_all(config)?;
    OpenLocationTarget::export_all(config)?;
    OpenLocationFailedParams::export_all(config)?;
    SkillFolderConflictParams::export_all(config)?;
    TaskBaseBranchNotFoundParams::export_all(config)?;
    PublicError::export_all(config)?;
    ContractError::export_all(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ContractError, EmptyErrorParams, OpenLocationFailedParams, OpenLocationTarget, PublicError,
        RequestId, SkillFolderConflictParams, TaskBaseBranchNotFoundParams,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use uuid::uuid;

    #[test]
    fn serializes_a_transport_neutral_error_without_a_message_or_envelope() {
        let error = ContractError {
            error: PublicError::ProjectNotFound(EmptyErrorParams {}),
            request_id: RequestId::from_uuid(uuid!("550e8400-e29b-41d4-a716-446655440000")),
        };

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({
                "code": "project_not_found",
                "params": {},
                "requestId": "550e8400-e29b-41d4-a716-446655440000",
            })
        );
    }

    /// Builds one representative value for every public error variant.
    ///
    /// The trailing match is exhaustive on purpose: adding a `PublicError` variant fails
    /// compilation here until the sample list is updated, so code/serde drift cannot skip a
    /// newly added variant.
    fn public_error_samples() -> Vec<PublicError> {
        let empty = EmptyErrorParams {};
        let samples = vec![
            PublicError::InternalError(empty),
            PublicError::InvalidRequest(empty),
            PublicError::SkillNameBlank(empty),
            PublicError::SkillNameInvalid(empty),
            PublicError::SkillNameTooLong(empty),
            PublicError::SkillDescriptionBlank(empty),
            PublicError::SkillDescriptionTooLarge(empty),
            PublicError::SkillNameConflict(empty),
            PublicError::SkillNotFound(empty),
            PublicError::AgentNameBlank(empty),
            PublicError::AgentNameConflict(empty),
            PublicError::AgentNotFound(empty),
            PublicError::PluginNotFound(empty),
            PublicError::PluginDisabled(empty),
            PublicError::ProjectNotFound(empty),
            PublicError::TaskNotFound(empty),
            PublicError::ResourceInUse(empty),
            PublicError::WorktreeRequiresGitRepository(empty),
            PublicError::TaskBaseBranchRequired(empty),
            PublicError::TaskBaseBranchNotFound(TaskBaseBranchNotFoundParams {
                branch_name: "main".to_string(),
            }),
            PublicError::WorktreeNotFound(empty),
            PublicError::TaskDiffBaselineUnavailable(empty),
            PublicError::TaskDiffCommitMessageBlank(empty),
            PublicError::TaskDiffTooLarge(empty),
            PublicError::TaskDiffStale(empty),
            PublicError::TaskDiffCommentNotFound(empty),
            PublicError::TaskDiffCommentInvalid(empty),
            PublicError::TaskDiffCommentConflict(empty),
            PublicError::SessionNotFound(empty),
            PublicError::AgentCliNotFound(empty),
            PublicError::AgentRuntimeUnavailable(empty),
            PublicError::SessionBusy(empty),
            PublicError::SessionStopped(empty),
            PublicError::SessionLoadUnsupported(empty),
            PublicError::SessionHistoryDegraded(empty),
            PublicError::SessionAgentUnchanged(empty),
            PublicError::PermissionRequestNotPending(empty),
            PublicError::PermissionOptionInvalid(empty),
            PublicError::PromptEmpty(empty),
            PublicError::PromptTooLarge(empty),
            PublicError::TaskWorktreeUnavailable(empty),
            PublicError::TaskProjectRootUnavailable(empty),
            PublicError::FileSystemPathNotFound(empty),
            PublicError::SpecDocumentNotFound(empty),
            PublicError::WorktreeRootNotAbsolute(empty),
            PublicError::WorktreeRootNotDirectory(empty),
            PublicError::OpenLocationFailed(OpenLocationFailedParams {
                target: OpenLocationTarget::Explorer,
            }),
            PublicError::SkillManifestMissing(empty),
            PublicError::SkillManifestInvalid(empty),
            PublicError::SkillManifestNameBlank(empty),
            PublicError::SkillManifestDescriptionBlank(empty),
            PublicError::SkillManifestNameInvalid(empty),
            PublicError::SkillFolderConflict(SkillFolderConflictParams {
                name: "review".to_string(),
            }),
            PublicError::SkillManifestNotFound(empty),
            PublicError::SkillManifestTooLarge(empty),
            PublicError::TooManySkills(empty),
            PublicError::ArchiveFormatUnsupported(empty),
            PublicError::ArchiveFormatMismatch(empty),
            PublicError::ArchiveCorrupt(empty),
            PublicError::ArchiveEncryptedUnsupported(empty),
            PublicError::ArchiveSpecialEntryUnsupported(empty),
            PublicError::ArchivePathEncodingInvalid(empty),
            PublicError::ArchivePathCaseConflict(empty),
            PublicError::PathSegmentTooLong(empty),
            PublicError::PathTooLong(empty),
            PublicError::PathTooDeep(empty),
            PublicError::ArchiveExpansionRatioExceeded(empty),
            PublicError::ImportPreparationTimeout(empty),
            PublicError::ImportSessionExpired(empty),
            PublicError::ImportSessionCancelled(empty),
            PublicError::ImportSessionCommitInProgress(empty),
            PublicError::ImportSessionAlreadyCommitted(empty),
            PublicError::SkillStorageInconsistent(empty),
            PublicError::WorkflowNameBlank(empty),
            PublicError::WorkflowNameConflict(empty),
            PublicError::WorkflowNotFound(empty),
            PublicError::WorkflowSnapshotNotFound(empty),
            PublicError::WorkflowVersionAlreadyExists(empty),
            PublicError::WorkflowVersionInvalid(empty),
            PublicError::WorkflowVersionReserved(empty),
            PublicError::WorkflowCannotDeleteDraft(empty),
            PublicError::WorkflowCannotDeleteActiveVersion(empty),
            PublicError::WorkflowActiveRuns(empty),
            PublicError::WorkflowCannotRollbackToDraft(empty),
            PublicError::WorkflowCannotActivateDraft(empty),
            PublicError::WorkflowSnapshotInUse(empty),
            PublicError::WorkflowNoPublishedSnapshot(empty),
            PublicError::WorkflowRunCannotUseDraftSnapshot(empty),
            PublicError::WorkflowRunNotFound(empty),
            PublicError::WorkflowRunActive(empty),
        ];

        for error in &samples {
            match error {
                PublicError::InternalError(_)
                | PublicError::InvalidRequest(_)
                | PublicError::SkillNameBlank(_)
                | PublicError::SkillNameInvalid(_)
                | PublicError::SkillNameTooLong(_)
                | PublicError::SkillDescriptionBlank(_)
                | PublicError::SkillDescriptionTooLarge(_)
                | PublicError::SkillNameConflict(_)
                | PublicError::SkillNotFound(_)
                | PublicError::AgentNameBlank(_)
                | PublicError::AgentNameConflict(_)
                | PublicError::AgentNotFound(_)
                | PublicError::PluginNotFound(_)
                | PublicError::PluginDisabled(_)
                | PublicError::ProjectNotFound(_)
                | PublicError::TaskNotFound(_)
                | PublicError::ResourceInUse(_)
                | PublicError::WorktreeRequiresGitRepository(_)
                | PublicError::TaskBaseBranchRequired(_)
                | PublicError::TaskBaseBranchNotFound(_)
                | PublicError::WorktreeNotFound(_)
                | PublicError::TaskDiffBaselineUnavailable(_)
                | PublicError::TaskDiffCommitMessageBlank(_)
                | PublicError::TaskDiffTooLarge(_)
                | PublicError::TaskDiffStale(_)
                | PublicError::TaskDiffCommentNotFound(_)
                | PublicError::TaskDiffCommentInvalid(_)
                | PublicError::TaskDiffCommentConflict(_)
                | PublicError::SessionNotFound(_)
                | PublicError::AgentCliNotFound(_)
                | PublicError::AgentRuntimeUnavailable(_)
                | PublicError::SessionBusy(_)
                | PublicError::SessionStopped(_)
                | PublicError::SessionLoadUnsupported(_)
                | PublicError::SessionHistoryDegraded(_)
                | PublicError::SessionAgentUnchanged(_)
                | PublicError::PermissionRequestNotPending(_)
                | PublicError::PermissionOptionInvalid(_)
                | PublicError::PromptEmpty(_)
                | PublicError::PromptTooLarge(_)
                | PublicError::TaskWorktreeUnavailable(_)
                | PublicError::TaskProjectRootUnavailable(_)
                | PublicError::FileSystemPathNotFound(_)
                | PublicError::SpecDocumentNotFound(_)
                | PublicError::WorktreeRootNotAbsolute(_)
                | PublicError::WorktreeRootNotDirectory(_)
                | PublicError::OpenLocationFailed(_)
                | PublicError::SkillManifestMissing(_)
                | PublicError::SkillManifestInvalid(_)
                | PublicError::SkillManifestNameBlank(_)
                | PublicError::SkillManifestDescriptionBlank(_)
                | PublicError::SkillManifestNameInvalid(_)
                | PublicError::SkillFolderConflict(_)
                | PublicError::SkillManifestNotFound(_)
                | PublicError::SkillManifestTooLarge(_)
                | PublicError::TooManySkills(_)
                | PublicError::ArchiveFormatUnsupported(_)
                | PublicError::ArchiveFormatMismatch(_)
                | PublicError::ArchiveCorrupt(_)
                | PublicError::ArchiveEncryptedUnsupported(_)
                | PublicError::ArchiveSpecialEntryUnsupported(_)
                | PublicError::ArchivePathEncodingInvalid(_)
                | PublicError::ArchivePathCaseConflict(_)
                | PublicError::PathSegmentTooLong(_)
                | PublicError::PathTooLong(_)
                | PublicError::PathTooDeep(_)
                | PublicError::ArchiveExpansionRatioExceeded(_)
                | PublicError::ImportPreparationTimeout(_)
                | PublicError::ImportSessionExpired(_)
                | PublicError::ImportSessionCancelled(_)
                | PublicError::ImportSessionCommitInProgress(_)
                | PublicError::ImportSessionAlreadyCommitted(_)
                | PublicError::SkillStorageInconsistent(_)
                | PublicError::WorkflowNameBlank(_)
                | PublicError::WorkflowNameConflict(_)
                | PublicError::WorkflowNotFound(_)
                | PublicError::WorkflowSnapshotNotFound(_)
                | PublicError::WorkflowVersionAlreadyExists(_)
                | PublicError::WorkflowVersionInvalid(_)
                | PublicError::WorkflowVersionReserved(_)
                | PublicError::WorkflowCannotDeleteDraft(_)
                | PublicError::WorkflowCannotDeleteActiveVersion(_)
                | PublicError::WorkflowActiveRuns(_)
                | PublicError::WorkflowCannotRollbackToDraft(_)
                | PublicError::WorkflowCannotActivateDraft(_)
                | PublicError::WorkflowSnapshotInUse(_)
                | PublicError::WorkflowNoPublishedSnapshot(_)
                | PublicError::WorkflowRunCannotUseDraftSnapshot(_)
                | PublicError::WorkflowRunNotFound(_)
                | PublicError::WorkflowRunActive(_)
                | PublicError::WorkflowRunGraphParse(_)
                | PublicError::WorkflowRunValidation(_)
                | PublicError::WorkflowSkillNotFound(_)
                | PublicError::WorkflowRoleNotFound(_)
                | PublicError::WorkflowRunStartFailed(_)
                | PublicError::WorkflowRunNotRestartable(_)
                | PublicError::WorkflowRunNotEditable(_) => {}
            }
        }

        samples
    }

    /// Verifies the manually exposed code cannot drift from Serde's tagged representation.
    #[test]
    fn public_error_codes_match_serde_tags_for_every_variant() {
        let samples = public_error_samples();
        assert_eq!(samples.len(), 90);

        for error in samples {
            let serialized = serde_json::to_value(&error).unwrap();

            assert_eq!(
                serialized.get("code").and_then(serde_json::Value::as_str),
                Some(error.code()),
                "code mismatch for {error:?}"
            );
        }
    }
}

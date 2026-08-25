use ora_application::{ApplicationError, SkillImportError};
use ora_contracts::{
    ContractError, EmptyErrorParams, PublicError, RequestId, SkillFolderConflictParams,
};
use ora_plugin_lifecycle::PluginLifecycleError;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

type SharedError = Arc<dyn Error + Send + Sync + 'static>;

/// Classifies public failures without coupling the shared runtime to adapter-specific status codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClassification {
    InvalidRequest,
    PayloadTooLarge,
    NotFound,
    Conflict,
    Unprocessable,
    Internal,
}

/// Preserves internal diagnostics while exposing only a typed public error to adapters.
#[derive(Clone, Debug)]
pub struct BackendError {
    classification: ErrorClassification,
    public_error: PublicError,
    context: String,
    source: Option<SharedError>,
}

impl BackendError {
    /// Creates a semantic backend failure that has no lower-level source.
    pub fn new(
        classification: ErrorClassification,
        public_error: PublicError,
        context: impl Into<String>,
    ) -> Self {
        Self {
            classification,
            public_error,
            context: context.into(),
            source: None,
        }
    }

    /// Creates an internal failure while retaining its concrete source chain.
    pub fn internal(context: &'static str, source: impl Error + Send + Sync + 'static) -> Self {
        Self::with_source(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            context,
            source,
        )
    }

    /// Creates an internal failure from an already boxed source-chain boundary.
    pub fn internal_boxed(
        context: &'static str,
        source: Box<dyn Error + Send + Sync + 'static>,
    ) -> Self {
        Self {
            classification: ErrorClassification::Internal,
            public_error: PublicError::InternalError(EmptyErrorParams {}),
            context: context.to_string(),
            source: Some(Arc::from(source)),
        }
    }

    /// Creates a classified semantic failure while retaining a lower-level source.
    pub fn with_source(
        classification: ErrorClassification,
        public_error: PublicError,
        context: &'static str,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            classification,
            public_error,
            context: context.to_string(),
            source: Some(Arc::new(source)),
        }
    }

    /// Returns the category an adapter maps into native status and logging semantics.
    pub const fn classification(&self) -> ErrorClassification {
        self.classification
    }

    /// Returns the strongly typed public error without exposing internal diagnostics.
    pub const fn public_error(&self) -> &PublicError {
        &self.public_error
    }

    /// Builds the public payload using the adapter-owned request identifier.
    pub fn contract_error(&self, request_id: RequestId) -> ContractError {
        ContractError {
            error: self.public_error.clone(),
            request_id,
        }
    }
}

impl fmt::Display for BackendError {
    /// Formats only the semantic context added by the backend layer.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl Error for BackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as &dyn Error)
    }
}

impl From<PluginLifecycleError> for BackendError {
    /// Maps lifecycle semantics to stable adapter classifications while retaining diagnostics.
    fn from(error: PluginLifecycleError) -> Self {
        let (classification, public_error, context) = match &error {
            PluginLifecycleError::PluginNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
                "installed plugin was not found",
            ),
            PluginLifecycleError::PluginDisabled { .. } => (
                ErrorClassification::Conflict,
                PublicError::PluginDisabled(EmptyErrorParams {}),
                "plugin must be enabled before activation",
            ),
            PluginLifecycleError::InvalidConfigurationDeclaration { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::PluginConfigurationDeclarationInvalid(EmptyErrorParams {}),
                "plugin configuration declaration is invalid",
            ),
            PluginLifecycleError::NoProcess { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "plugin kind has no process to activate",
            ),
            PluginLifecycleError::Repository(_)
            | PluginLifecycleError::RuntimeStop { .. }
            | PluginLifecycleError::PackageRemoval { .. }
            | PluginLifecycleError::UninstallStaging { .. } => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "plugin lifecycle operation failed",
            ),
        };
        Self::with_source(classification, public_error, context, error)
    }
}

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
            ApplicationError::SkillInUse => (
                ErrorClassification::Conflict,
                PublicError::ResourceInUse(EmptyErrorParams {}),
                "skill is referenced by Workspace desired state",
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
            ApplicationError::SkillImport(error) => {
                let params = EmptyErrorParams {};
                match error {
                    SkillImportError::SkillManifestNotFound => (
                        ErrorClassification::Unprocessable,
                        PublicError::SkillManifestNotFound(params),
                        "skill import manifest was not found",
                    ),
                    SkillImportError::TooManySkills { .. } => (
                        ErrorClassification::Unprocessable,
                        PublicError::TooManySkills(params),
                        "skill import has too many manifests",
                    ),
                    SkillImportError::TooManyFiles { .. }
                    | SkillImportError::TooManyEntries { .. } => (
                        ErrorClassification::Unprocessable,
                        PublicError::InvalidRequest(params),
                        "skill import has too many source entries",
                    ),
                    SkillImportError::DuplicateSkillNames { .. } => (
                        ErrorClassification::Unprocessable,
                        PublicError::InvalidRequest(params),
                        "skill import has duplicate names",
                    ),
                    SkillImportError::ArchiveFormatUnsupported => (
                        ErrorClassification::InvalidRequest,
                        PublicError::ArchiveFormatUnsupported(params),
                        "skill archive format is unsupported",
                    ),
                    SkillImportError::ArchiveFormatMismatch => (
                        ErrorClassification::Unprocessable,
                        PublicError::ArchiveFormatMismatch(params),
                        "skill archive format does not match content",
                    ),
                    SkillImportError::ArchiveCorrupt => (
                        ErrorClassification::Unprocessable,
                        PublicError::ArchiveCorrupt(params),
                        "skill archive is corrupt",
                    ),
                    SkillImportError::ArchiveTooLarge
                    | SkillImportError::TotalBytesExceeded
                    | SkillImportError::ArchiveExpansionRatioExceeded => (
                        ErrorClassification::PayloadTooLarge,
                        PublicError::ArchiveExpansionRatioExceeded(params),
                        "skill import exceeds source limits",
                    ),
                    SkillImportError::ArchiveEncryptedUnsupported => (
                        ErrorClassification::Unprocessable,
                        PublicError::ArchiveEncryptedUnsupported(params),
                        "encrypted skill archive is unsupported",
                    ),
                    SkillImportError::ArchiveSpecialEntryUnsupported => (
                        ErrorClassification::Unprocessable,
                        PublicError::ArchiveSpecialEntryUnsupported(params),
                        "skill archive contains a special entry",
                    ),
                    SkillImportError::ArchivePathEncodingInvalid => (
                        ErrorClassification::Unprocessable,
                        PublicError::ArchivePathEncodingInvalid(params),
                        "skill archive path encoding is invalid",
                    ),
                    SkillImportError::ArchivePathCaseConflict => (
                        ErrorClassification::Unprocessable,
                        PublicError::ArchivePathCaseConflict(params),
                        "skill source paths conflict",
                    ),
                    SkillImportError::PathSegmentTooLong => (
                        ErrorClassification::Unprocessable,
                        PublicError::PathSegmentTooLong(params),
                        "skill source path segment is too long",
                    ),
                    SkillImportError::PathTooLong => (
                        ErrorClassification::Unprocessable,
                        PublicError::PathTooLong(params),
                        "skill source path is too long",
                    ),
                    SkillImportError::PathTooDeep => (
                        ErrorClassification::Unprocessable,
                        PublicError::PathTooDeep(params),
                        "skill source path is too deep",
                    ),
                    SkillImportError::UnsafePath => (
                        ErrorClassification::Unprocessable,
                        PublicError::InvalidRequest(params),
                        "skill source path is unsafe",
                    ),
                    SkillImportError::PreparationTimeout => (
                        ErrorClassification::Unprocessable,
                        PublicError::ImportPreparationTimeout(params),
                        "skill import preparation timed out",
                    ),
                    SkillImportError::SessionNotFound { .. } | SkillImportError::SessionExpired => {
                        (
                            ErrorClassification::NotFound,
                            PublicError::ImportSessionExpired(params),
                            "skill import session is unavailable",
                        )
                    }
                    SkillImportError::SessionCancelled => (
                        ErrorClassification::Conflict,
                        PublicError::ImportSessionCancelled(params),
                        "skill import session was cancelled",
                    ),
                    SkillImportError::CommitInProgress => (
                        ErrorClassification::Conflict,
                        PublicError::ImportSessionCommitInProgress(params),
                        "skill import commit is in progress",
                    ),
                    SkillImportError::AlreadyCommitted => (
                        ErrorClassification::Conflict,
                        PublicError::ImportSessionAlreadyCommitted(params),
                        "skill import session was already committed",
                    ),
                    SkillImportError::DecisionMissing { .. } => (
                        ErrorClassification::InvalidRequest,
                        PublicError::InvalidRequest(params),
                        "skill import decisions are incomplete",
                    ),
                    SkillImportError::SourceUnavailable { .. }
                    | SkillImportError::Storage { .. }
                    | SkillImportError::Repository { .. }
                    | SkillImportError::Internal { .. } => (
                        ErrorClassification::Internal,
                        PublicError::InternalError(params),
                        "skill import operation failed",
                    ),
                }
            }
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
            ApplicationError::TaskDiffCommitMessageBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::TaskDiffCommitMessageBlank(EmptyErrorParams {}),
                "task diff commit message must not be blank",
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
            ApplicationError::SkillRepository { .. }
            | ApplicationError::SkillStorage { .. }
            | ApplicationError::AgentDefinitionRepository { .. }
            | ApplicationError::ProjectRepository { .. }
            | ApplicationError::TaskRepository { .. }
            | ApplicationError::TaskWorkspaceIdExhausted { .. }
            | ApplicationError::TaskWorktreeRootUnavailable
            | ApplicationError::TaskFilesystem { .. }
            | ApplicationError::TaskWorktreeProvisioner { .. }
            | ApplicationError::TaskDiff { .. }
            | ApplicationError::WorktreeRepository { .. }
            | ApplicationError::SessionRepository { .. }
            | ApplicationError::UserConfigRepository { .. }
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
            ApplicationError::WorkflowRunGraphParse(_) => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunGraphParse(EmptyErrorParams {}),
                "workflow graph is invalid",
            ),
            ApplicationError::WorkflowRunValidation(_) => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunValidation(EmptyErrorParams {}),
                "workflow run is not executable",
            ),
            ApplicationError::WorkflowSkillNotFound { .. } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowSkillNotFound(EmptyErrorParams {}),
                "workflow skill not found",
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

#[cfg(test)]
mod tests {
    use super::{BackendError, ErrorClassification};
    use ora_application::{ApplicationError, RepositoryError, SkillImportError};
    use ora_contracts::{EmptyErrorParams, PublicError, SkillFolderConflictParams};
    use pretty_assertions::assert_eq;
    use std::error::Error;

    /// Verifies non-Git roots retain the stable bad-request contract used by runtime adapters.
    #[test]
    fn maps_semantics_without_inspecting_the_source_chain() {
        let error = BackendError::from(ApplicationError::TaskWorktreeRequiresGitRepository);

        assert_eq!(error.classification(), ErrorClassification::InvalidRequest);
        assert_eq!(
            error.public_error(),
            &PublicError::WorktreeRequiresGitRepository(EmptyErrorParams {})
        );
        assert_eq!(
            error.source().map(ToString::to_string),
            Some("worktree mode requires a Git repository".to_string())
        );
    }

    /// Verifies skill conflicts expose only bounded typed parameters.
    #[test]
    fn maps_skill_import_semantics_to_public_contracts() {
        let conflict = BackendError::from(ApplicationError::SkillFolderConflict {
            name: "grilling".to_string(),
        });

        assert_eq!(
            (conflict.classification(), conflict.public_error().clone()),
            (
                ErrorClassification::Conflict,
                PublicError::SkillFolderConflict(SkillFolderConflictParams {
                    name: "grilling".to_string(),
                }),
            )
        );
    }

    /// Verifies wrapped skill import failures map through the main application error projection.
    #[test]
    fn maps_wrapped_skill_import_errors() {
        let error = BackendError::from(ApplicationError::SkillImport(
            SkillImportError::ArchiveCorrupt,
        ));

        assert_eq!(
            (error.classification(), error.public_error().clone()),
            (
                ErrorClassification::Unprocessable,
                PublicError::ArchiveCorrupt(EmptyErrorParams {})
            )
        );
    }

    /// Verifies duplicate Role names remain a public conflict instead of an internal repository error.
    #[test]
    fn maps_agent_name_conflicts_to_the_public_contract() {
        let error = BackendError::from(ApplicationError::AgentDefinitionNameConflict {
            namespace: "local".to_string(),
            name: "reviewer".to_string(),
        });

        assert_eq!(
            (error.classification(), error.public_error().clone()),
            (
                ErrorClassification::Conflict,
                PublicError::AgentNameConflict(EmptyErrorParams {})
            )
        );
    }

    /// Verifies a blank task commit message is reported as a client-correctable request error.
    #[test]
    fn maps_blank_task_commit_messages_to_invalid_request() {
        let error = BackendError::from(ApplicationError::TaskDiffCommitMessageBlank);

        assert_eq!(
            (error.classification(), error.public_error().clone()),
            (
                ErrorClassification::InvalidRequest,
                PublicError::TaskDiffCommitMessageBlank(EmptyErrorParams {})
            )
        );
    }

    /// Verifies task-diff infrastructure failures retain their concrete source chain.
    #[test]
    fn retains_task_diff_source_chain_through_the_backend_projection() {
        let application_error = ApplicationError::TaskDiff {
            source: Box::new(std::io::Error::other("git process failed")),
        };
        let backend_error = BackendError::from(application_error);

        assert_eq!(
            backend_error.source().map(ToString::to_string),
            Some("task diff operation failed".to_string())
        );
        assert_eq!(
            backend_error
                .source()
                .and_then(Error::source)
                .map(ToString::to_string),
            Some("git process failed".to_string())
        );
    }

    /// Verifies workflow domain failures preserve their distinct public contract codes.
    #[test]
    fn maps_workflow_failures_to_distinct_public_errors() {
        let cases = [
            (
                ApplicationError::WorkflowNameBlank,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowNameBlank(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowNameConflict {
                    namespace: "local".to_string(),
                    name: "review".to_string(),
                },
                ErrorClassification::Conflict,
                PublicError::WorkflowNameConflict(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowNotFound {
                    workflow_id: "workflow-1".to_string(),
                },
                ErrorClassification::NotFound,
                PublicError::WorkflowNotFound(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowSnapshotNotFound {
                    workflow_id: "workflow-1".to_string(),
                    version: "v1".to_string(),
                },
                ErrorClassification::NotFound,
                PublicError::WorkflowSnapshotNotFound(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowVersionAlreadyExists {
                    workflow_id: "workflow-1".to_string(),
                    version: "v1".to_string(),
                },
                ErrorClassification::Conflict,
                PublicError::WorkflowVersionAlreadyExists(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowVersionInvalid,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowVersionInvalid(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowVersionReserved,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowVersionReserved(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowCannotDeleteDraft,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotDeleteDraft(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowCannotDeleteActiveVersion,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotDeleteActiveVersion(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowActiveRuns,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowActiveRuns(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowCannotRollbackToDraft,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotRollbackToDraft(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowCannotActivateDraft,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowCannotActivateDraft(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowSnapshotInUse,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowSnapshotInUse(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowNoPublishedSnapshot,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowNoPublishedSnapshot(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowRunCannotUseDraftSnapshot,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunCannotUseDraftSnapshot(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowRunNotFound {
                    run_id: "run-1".to_string(),
                },
                ErrorClassification::NotFound,
                PublicError::WorkflowRunNotFound(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowRunActive,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowRunActive(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowNodeNotFound {
                    node_id: "node-1".to_string(),
                },
                ErrorClassification::NotFound,
                PublicError::WorkflowNodeNotFound(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowNodeNotAwaitingInput {
                    node_id: "node-1".to_string(),
                },
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {}),
            ),
        ];

        for (application_error, classification, public_error) in cases {
            let backend_error = BackendError::from(application_error);

            assert_eq!(backend_error.classification(), classification);
            assert_eq!(backend_error.public_error(), &public_error);
        }
    }

    #[test]
    fn retains_repository_source_chain_through_the_backend_projection() {
        let database_error = std::io::Error::other("database connection closed");
        let repository_error = RepositoryError::new(database_error);
        let application_error = ApplicationError::ProjectRepository {
            source: repository_error,
        };
        let backend_error = BackendError::from(application_error);

        let mut chain = Vec::new();
        let mut current: Option<&(dyn Error + 'static)> = Some(&backend_error);
        while let Some(error) = current {
            chain.push(error.to_string());
            current = error.source();
        }

        assert_eq!(
            chain,
            vec![
                "application operation failed",
                "project repository operation failed",
                "repository operation failed",
                "database connection closed",
            ]
        );
    }

    /// Verifies missing base branches remain actionable and retain their selected ref name.
    #[test]
    fn exposes_missing_base_branches_as_a_stable_bad_request() {
        let error = BackendError::from(ApplicationError::TaskBaseBranchNotFound {
            branch_name: "ghost-branch".to_string(),
        });

        assert_eq!(error.classification(), ErrorClassification::InvalidRequest);
        assert_eq!(
            error.public_error(),
            &PublicError::TaskBaseBranchNotFound(ora_contracts::TaskBaseBranchNotFoundParams {
                branch_name: "ghost-branch".to_string(),
            })
        );
    }
}

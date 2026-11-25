mod application;
mod skill_import;

use ora_agent_runtime::{ErrorClassification as RuntimeErrorClassification, RuntimeError};
use ora_contracts::{
    ContractError, EmptyErrorParams, PluginLogLevelsFileParams, PublicError, RequestId,
};
use ora_plugin_lifecycle::{PluginLifecycleError, PluginLogLevelPersistError};
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
    /// Access was refused by the host rather than by Ora's own request validation.
    ///
    /// Kept apart from `Conflict` so refusals do not dilute the conflict bucket, while still
    /// logging above the successful-request stream that `InvalidRequest` shares.
    Forbidden,
    /// The host environment could not carry out an action Ora delegated to it.
    ///
    /// Distinct from `Internal`, which blames Ora's own runtime: a missing editor or a shell that
    /// refuses a handoff is the operator's environment, and reporting it at `ERROR` would compete
    /// with genuine backend faults.
    HostUnavailable,
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

    /// Creates an invalid-request failure for malformed network proxy settings.
    pub fn invalid_proxy_settings(context: impl Into<String>) -> Self {
        Self::new(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            context,
        )
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

impl From<RuntimeError> for BackendError {
    /// Carries a runtime failure across unchanged: same class, public error, context, and source.
    fn from(error: RuntimeError) -> Self {
        let (classification, public_error, context, source) = error.into_parts();
        Self {
            classification: match classification {
                RuntimeErrorClassification::InvalidRequest => ErrorClassification::InvalidRequest,
                RuntimeErrorClassification::PayloadTooLarge => ErrorClassification::PayloadTooLarge,
                RuntimeErrorClassification::NotFound => ErrorClassification::NotFound,
                RuntimeErrorClassification::Forbidden => ErrorClassification::Forbidden,
                RuntimeErrorClassification::HostUnavailable => ErrorClassification::HostUnavailable,
                RuntimeErrorClassification::Conflict => ErrorClassification::Conflict,
                RuntimeErrorClassification::Unprocessable => ErrorClassification::Unprocessable,
                RuntimeErrorClassification::Internal => ErrorClassification::Internal,
            },
            public_error,
            context,
            source,
        }
    }
}

impl From<BackendError> for RuntimeError {
    /// Hands a Desktop host failure to the runtime without losing anything it will report back.
    fn from(error: BackendError) -> Self {
        let BackendError {
            classification,
            public_error,
            context,
            source,
        } = error;
        RuntimeError::from_parts(
            match classification {
                ErrorClassification::InvalidRequest => RuntimeErrorClassification::InvalidRequest,
                ErrorClassification::PayloadTooLarge => RuntimeErrorClassification::PayloadTooLarge,
                ErrorClassification::NotFound => RuntimeErrorClassification::NotFound,
                ErrorClassification::Forbidden => RuntimeErrorClassification::Forbidden,
                ErrorClassification::HostUnavailable => RuntimeErrorClassification::HostUnavailable,
                ErrorClassification::Conflict => RuntimeErrorClassification::Conflict,
                ErrorClassification::Unprocessable => RuntimeErrorClassification::Unprocessable,
                ErrorClassification::Internal => RuntimeErrorClassification::Internal,
            },
            public_error,
            context,
            source,
        )
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
            PluginLifecycleError::LogLevelPersistence { source, .. } => match source {
                PluginLogLevelPersistError::ReadOnly { path } => (
                    ErrorClassification::Conflict,
                    PublicError::PluginLogLevelsReadOnly(PluginLogLevelsFileParams {
                        path: path.to_string_lossy().into_owned(),
                    }),
                    "plugin log level settings require repair",
                ),
                PluginLogLevelPersistError::Write { path, .. } => (
                    ErrorClassification::HostUnavailable,
                    PublicError::PluginLogLevelsWriteFailed(PluginLogLevelsFileParams {
                        path: path.to_string_lossy().into_owned(),
                    }),
                    "plugin log level settings could not be persisted",
                ),
            },
            PluginLifecycleError::RuntimeStop { .. }
            | PluginLifecycleError::PackageRemoval { .. }
            | PluginLifecycleError::UninstallStaging { .. }
            | PluginLifecycleError::LogWriterActive { .. } => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "plugin lifecycle operation failed",
            ),
        };
        Self::with_source(classification, public_error, context, error)
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendError, ErrorClassification};
    use ora_application::{
        ApplicationError, INVALID_START_OPTION_DETAIL, MISSING_REQUIRED_START_DETAIL,
        RepositoryError, SkillImportError, WorkflowValidationError,
    };
    use ora_contracts::{
        EmptyErrorParams, PublicError, SkillFolderConflictParams, WorkflowRunInputInvalidParams,
        WorkflowSnapshotIncompatibleWithResumeParams,
    };
    use pretty_assertions::assert_eq;
    use std::error::Error;

    /// Verifies a failure crossing the runtime boundary in both directions comes back unchanged.
    ///
    /// Desktop hands host failures (an unavailable Workspace) to the shared runtime and receives
    /// them back as `BackendError`; anything lost on the way would change what the client sees.
    #[test]
    fn round_trips_through_the_runtime_error_without_loss() {
        let original = BackendError::with_source(
            ErrorClassification::Conflict,
            PublicError::WorkspaceUnavailable(EmptyErrorParams {}),
            "workspace is unavailable",
            RepositoryError::from_message("row was not found"),
        );

        let returned = BackendError::from(ora_agent_runtime::RuntimeError::from(original.clone()));

        assert_eq!(
            (
                returned.classification(),
                returned.public_error().clone(),
                returned.to_string(),
                returned.source().map(ToString::to_string),
            ),
            (
                original.classification(),
                original.public_error().clone(),
                original.to_string(),
                original.source().map(ToString::to_string),
            ),
        );
    }

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

    /// Verifies run-input rejections stay user-actionable and name the exact variable.
    #[test]
    fn maps_run_input_rejections_with_the_variable_and_reason() {
        let error = BackendError::from(ApplicationError::WorkflowRunInputInvalid {
            variable: "count".to_string(),
            reason: "value does not match the declared type number".to_string(),
        });

        assert_eq!(error.classification(), ErrorClassification::InvalidRequest);
        assert_eq!(
            error.public_error().clone(),
            PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                variable: "count".to_string(),
                reason: "value does not match the declared type number".to_string(),
            })
        );
    }

    /// Verifies starting without a required Start value names the missing variable instead of a
    /// generic validation failure.
    #[test]
    fn maps_a_missing_required_start_variable_with_its_name() {
        let error = BackendError::from(ApplicationError::WorkflowRunValidation(
            WorkflowValidationError::MissingRequiredStartVariable {
                name: "brief".to_string(),
            },
        ));

        assert_eq!(error.classification(), ErrorClassification::InvalidRequest);
        assert_eq!(
            error.public_error().clone(),
            PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                variable: "brief".to_string(),
                reason: MISSING_REQUIRED_START_DETAIL.to_string(),
            })
        );
    }

    /// Verifies an out-of-option Start value names the variable instead of a generic failure,
    /// pinning the third start-validation branch of the same public contract.
    #[test]
    fn maps_an_invalid_start_variable_option_with_its_name() {
        let error = BackendError::from(ApplicationError::WorkflowRunValidation(
            WorkflowValidationError::InvalidStartVariableOption {
                name: "mode".to_string(),
            },
        ));

        assert_eq!(error.classification(), ErrorClassification::InvalidRequest);
        assert_eq!(
            error.public_error().clone(),
            PublicError::WorkflowRunInputInvalid(WorkflowRunInputInvalidParams {
                variable: "mode".to_string(),
                reason: INVALID_START_OPTION_DETAIL.to_string(),
            })
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

    /// Verifies a blank workspace commit message is reported as a client-correctable request error.
    #[test]
    fn maps_blank_workspace_commit_messages_to_invalid_request() {
        let error = BackendError::from(ApplicationError::WorkspaceDiffCommitMessageBlank);

        assert_eq!(
            (error.classification(), error.public_error().clone()),
            (
                ErrorClassification::InvalidRequest,
                PublicError::WorkspaceDiffCommitMessageBlank(EmptyErrorParams {})
            )
        );
    }

    /// Verifies workspace-diff infrastructure failures retain their concrete source chain.
    #[test]
    fn retains_workspace_diff_source_chain_through_the_backend_projection() {
        let application_error = ApplicationError::WorkspaceDiff {
            source: Box::new(std::io::Error::other("git process failed")),
        };
        let backend_error = BackendError::from(application_error);

        assert_eq!(
            backend_error.source().map(ToString::to_string),
            Some("workspace diff operation failed".to_string())
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
            (
                ApplicationError::WorkflowNodeNotDiagnosable,
                ErrorClassification::Conflict,
                PublicError::WorkflowNodeNotDiagnosable(EmptyErrorParams {}),
            ),
            (
                ApplicationError::WorkflowSnapshotIncompatibleWithResume {
                    reason: "node_missing:b".to_string(),
                },
                ErrorClassification::Conflict,
                PublicError::WorkflowSnapshotIncompatibleWithResume(
                    WorkflowSnapshotIncompatibleWithResumeParams {
                        reason: "node_missing:b".to_string(),
                    },
                ),
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

    /// File protection and write failures carry dedicated errors with the repair path.
    #[test]
    fn plugin_log_level_failures_expose_actionable_paths() {
        let path = std::path::PathBuf::from("plugins").join("log-levels.json");
        let params = ora_contracts::PluginLogLevelsFileParams {
            path: path.to_string_lossy().into_owned(),
        };
        for (source, classification, public) in [
            (
                ora_plugin_lifecycle::PluginLogLevelPersistError::ReadOnly { path: path.clone() },
                ErrorClassification::Conflict,
                PublicError::PluginLogLevelsReadOnly(params.clone()),
            ),
            (
                ora_plugin_lifecycle::PluginLogLevelPersistError::Write {
                    path: path.clone(),
                    source: std::io::Error::other("disk full"),
                },
                ErrorClassification::HostUnavailable,
                PublicError::PluginLogLevelsWriteFailed(params),
            ),
        ] {
            let error = BackendError::from(
                ora_plugin_lifecycle::PluginLifecycleError::LogLevelPersistence {
                    plugin_id: "official/example".to_owned(),
                    source,
                },
            );
            assert_eq!(
                (error.classification(), error.public_error()),
                (classification, &public)
            );
        }
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

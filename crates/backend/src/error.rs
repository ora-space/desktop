use ora_application::ApplicationError;
use ora_contracts::{
    ContractError, EmptyErrorParams, PublicError, RequestId, SkillFolderConflictParams,
    SkillUploadTooManyFilesParams,
};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

type SharedError = Arc<dyn Error + Send + Sync + 'static>;

/// Classifies public failures without coupling the shared runtime to HTTP status codes.
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

impl From<ApplicationError> for BackendError {
    /// Projects the highest application semantic variant and retains the complete source chain.
    fn from(error: ApplicationError) -> Self {
        let (classification, public_error, context) = match &error {
            ApplicationError::SkillNameBlank => (
                ErrorClassification::InvalidRequest,
                PublicError::SkillNameBlank(EmptyErrorParams {}),
                "skill name must not be blank",
            ),
            ApplicationError::SkillNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::SkillNotFound(EmptyErrorParams {}),
                "skill not found",
            ),
            ApplicationError::SkillUploadEmpty => (
                ErrorClassification::Unprocessable,
                PublicError::SkillUploadEmpty(EmptyErrorParams {}),
                "skill upload contained no files",
            ),
            ApplicationError::SkillUploadTooManyFiles { max_files } => (
                ErrorClassification::Unprocessable,
                PublicError::SkillUploadTooManyFiles(SkillUploadTooManyFilesParams {
                    max_files: *max_files,
                }),
                "skill upload contains too many files",
            ),
            ApplicationError::SkillUploadPathInvalid => (
                ErrorClassification::Unprocessable,
                PublicError::SkillUploadPathInvalid(EmptyErrorParams {}),
                "skill upload contains an unsafe path",
            ),
            ApplicationError::SkillUploadPathDuplicate => (
                ErrorClassification::Unprocessable,
                PublicError::SkillUploadPathDuplicate(EmptyErrorParams {}),
                "skill upload contains a duplicate path",
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
            ApplicationError::ProjectOccupied { .. } => (
                ErrorClassification::Conflict,
                PublicError::ProjectOccupied(EmptyErrorParams {}),
                "project is already occupied",
            ),
            ApplicationError::ProjectWorkContextNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::ProjectWorkContextNotFound(EmptyErrorParams {}),
                "project work context not found",
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
            ApplicationError::WorktreeNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::WorktreeNotFound(EmptyErrorParams {}),
                "worktree not found",
            ),
            ApplicationError::SessionNotFound { .. } => (
                ErrorClassification::NotFound,
                PublicError::SessionNotFound(EmptyErrorParams {}),
                "session not found",
            ),
            ApplicationError::SkillRepository { .. }
            | ApplicationError::SkillPackageStorage { .. }
            | ApplicationError::AgentDefinitionRepository { .. }
            | ApplicationError::ProjectRepository { .. }
            | ApplicationError::ProjectWorkContextRepository { .. }
            | ApplicationError::TaskRepository { .. }
            | ApplicationError::TaskWorktreeIdExhausted { .. }
            | ApplicationError::TaskWorktreeRootUnavailable
            | ApplicationError::TaskFilesystem { .. }
            | ApplicationError::TaskWorktreeProvisioner { .. }
            | ApplicationError::WorktreeRepository { .. }
            | ApplicationError::SessionRepository { .. } => (
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "application operation failed",
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
    use ora_application::{ApplicationError, RepositoryError};
    use ora_contracts::{
        EmptyErrorParams, PublicError, SkillFolderConflictParams, SkillUploadTooManyFilesParams,
    };
    use pretty_assertions::assert_eq;
    use std::error::Error;

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

    /// Verifies skill import validation and conflicts expose only bounded typed parameters.
    #[test]
    fn maps_skill_import_semantics_to_public_contracts() {
        let too_many =
            BackendError::from(ApplicationError::SkillUploadTooManyFiles { max_files: 1000 });
        let conflict = BackendError::from(ApplicationError::SkillFolderConflict {
            name: "grilling".to_string(),
        });

        assert_eq!(
            (
                too_many.classification(),
                too_many.public_error().clone(),
                conflict.classification(),
                conflict.public_error().clone(),
            ),
            (
                ErrorClassification::Unprocessable,
                PublicError::SkillUploadTooManyFiles(SkillUploadTooManyFilesParams {
                    max_files: 1000,
                }),
                ErrorClassification::Conflict,
                PublicError::SkillFolderConflict(SkillFolderConflictParams {
                    name: "grilling".to_string(),
                }),
            )
        );
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
}

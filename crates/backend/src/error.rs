use ora_application::ApplicationError;
use serde::Serialize;
use std::fmt;

/// Classifies backend failures without coupling the shared layer to HTTP status codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Internal,
}

/// Carries the stable public error code and message shared by every transport adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendError {
    #[serde(skip)]
    kind: BackendErrorKind,
    code: &'static str,
    message: String,
}

impl BackendError {
    /// Creates a backend error from explicit transport-neutral public fields.
    pub fn new(kind: BackendErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
        }
    }

    /// Returns the category an adapter can map into its native failure semantics.
    pub fn kind(&self) -> BackendErrorKind {
        self.kind
    }

    /// Returns the stable machine-readable public error code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable public error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BackendError {
    /// Formats the public message without exposing internal source diagnostics.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<ApplicationError> for BackendError {
    /// Normalizes application failures into one stable adapter-independent error contract.
    fn from(error: ApplicationError) -> Self {
        match error {
            ApplicationError::SkillNameBlank => Self::new(
                BackendErrorKind::BadRequest,
                "skill_name_blank",
                "skill name must not be blank",
            ),
            ApplicationError::SkillNameInvalid { name } => Self::new(
                BackendErrorKind::BadRequest,
                "skill_name_invalid",
                format!("invalid skill name: {name}"),
            ),
            ApplicationError::SkillNameTooLong => Self::new(
                BackendErrorKind::BadRequest,
                "skill_name_too_long",
                "skill name exceeds the single path segment limit",
            ),
            ApplicationError::SkillDescriptionBlank => Self::new(
                BackendErrorKind::BadRequest,
                "skill_description_blank",
                "skill description must not be blank",
            ),
            ApplicationError::SkillDescriptionTooLarge => Self::new(
                BackendErrorKind::BadRequest,
                "skill_description_too_large",
                "skill description exceeds 4096 bytes",
            ),
            ApplicationError::SkillNameConflict { name } => Self::new(
                BackendErrorKind::Conflict,
                "skill_name_conflict",
                format!("skill name already exists: {name}"),
            ),
            ApplicationError::SkillNotFound { skill_id } => Self::new(
                BackendErrorKind::NotFound,
                "skill_not_found",
                format!("skill not found: {skill_id}"),
            ),
            ApplicationError::SkillRepository { .. } => internal(
                "skill_repository_error",
                "skill repository operation failed",
            ),
            ApplicationError::SkillStorageInconsistent { name } => Self::new(
                BackendErrorKind::Internal,
                "skill_storage_inconsistent",
                format!("skill storage is inconsistent for: {name}"),
            ),
            ApplicationError::SkillStorage { .. } => {
                internal("skill_storage_error", "skill storage operation failed")
            }
            ApplicationError::SkillImport(error) => import_error(error),
            ApplicationError::AgentDefinitionNameBlank => Self::new(
                BackendErrorKind::BadRequest,
                "agent_name_blank",
                "agent definition name must not be blank",
            ),
            ApplicationError::AgentDefinitionNotFound { agent_id } => Self::new(
                BackendErrorKind::NotFound,
                "agent_not_found",
                format!("agent definition not found: {agent_id}"),
            ),
            ApplicationError::AgentDefinitionRepository { .. } => internal(
                "agent_repository_error",
                "agent repository operation failed",
            ),
            ApplicationError::ProjectNotFound { project_id } => Self::new(
                BackendErrorKind::NotFound,
                "project_not_found",
                format!("project not found: {project_id}"),
            ),
            ApplicationError::ProjectRepository { .. } => internal(
                "project_repository_error",
                "project repository operation failed",
            ),
            ApplicationError::ProjectOccupied { project_id } => Self::new(
                BackendErrorKind::Conflict,
                "project_occupied",
                format!("project is already occupied: {project_id}"),
            ),
            ApplicationError::ProjectWorkContextNotFound { surface, window_id } => Self::new(
                BackendErrorKind::NotFound,
                "project_work_context_not_found",
                format!("project work context not found for {surface}/{window_id}"),
            ),
            ApplicationError::ProjectWorkContextRepository { .. } => internal(
                "project_work_context_repository_error",
                "project work context repository operation failed",
            ),
            ApplicationError::TaskNotFound { task_id } => Self::new(
                BackendErrorKind::NotFound,
                "task_not_found",
                format!("task not found: {task_id}"),
            ),
            ApplicationError::TaskRepository { .. } => {
                internal("task_repository_error", "task repository operation failed")
            }
            ApplicationError::TaskWorktreeRequiresGitRepository => Self::new(
                BackendErrorKind::BadRequest,
                "worktree_requires_git_repository",
                "worktree mode requires a Git repository",
            ),
            ApplicationError::TaskWorktree { .. } => {
                internal("task_worktree_error", "task worktree operation failed")
            }
            ApplicationError::WorktreeNotFound { worktree_id } => Self::new(
                BackendErrorKind::NotFound,
                "worktree_not_found",
                format!("worktree not found: {worktree_id}"),
            ),
            ApplicationError::WorktreeRepository { .. } => internal(
                "worktree_repository_error",
                "worktree repository operation failed",
            ),
            ApplicationError::SessionNotFound { session_id } => Self::new(
                BackendErrorKind::NotFound,
                "session_not_found",
                format!("session not found: {session_id}"),
            ),
            ApplicationError::SessionRepository { .. } => internal(
                "session_repository_error",
                "session repository operation failed",
            ),
        }
    }
}

/// Builds a sanitized internal failure without leaking repository or filesystem diagnostics.
fn internal(code: &'static str, message: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::Internal, code, message)
}

/// Maps one import-session failure onto its stable public error code and message.
fn import_error(error: ora_application::SkillImportError) -> BackendError {
    use ora_application::SkillImportError;
    match error {
        SkillImportError::SkillManifestNotFound => bad_request(
            "skill_manifest_not_found",
            "no SKILL.md manifest was found in the source",
        ),
        SkillImportError::TooManySkills { max_skills } => bad_request(
            "too_many_skills",
            format!("source contains more than {max_skills} skills"),
        ),
        SkillImportError::TooManyFiles { max_files } => bad_request(
            "too_many_files",
            format!("one skill contains more than {max_files} files"),
        ),
        SkillImportError::DuplicateSkillNames { .. } => bad_request(
            "duplicate_skill_names",
            "multiple valid skills in one source declare the same name",
        ),
        SkillImportError::ArchiveFormatUnsupported => bad_request(
            "archive_format_unsupported",
            "unsupported archive format; allowed extensions: zip, skill, tar.gz, tgz",
        ),
        SkillImportError::ArchiveFormatMismatch => bad_request(
            "archive_format_mismatch",
            "archive contents do not match the requested format",
        ),
        SkillImportError::ArchiveCorrupt => {
            bad_request("archive_corrupt", "archive is corrupt or unreadable")
        }
        SkillImportError::ArchiveTooLarge => bad_request(
            "archive_too_large",
            "archive exceeds the maximum upload size",
        ),
        SkillImportError::ArchiveEncryptedUnsupported => bad_request(
            "archive_encrypted_unsupported",
            "encrypted archives are not supported",
        ),
        SkillImportError::ArchiveSpecialEntryUnsupported => bad_request(
            "archive_special_entry_unsupported",
            "archive contains a special entry that cannot be stored safely",
        ),
        SkillImportError::ArchivePathEncodingInvalid => bad_request(
            "archive_path_encoding_invalid",
            "archive entry path is not valid UTF-8",
        ),
        SkillImportError::ArchivePathCaseConflict => bad_request(
            "archive_path_case_conflict",
            "source paths conflict after portable case normalization",
        ),
        SkillImportError::PathSegmentTooLong => bad_request(
            "path_segment_too_long",
            "a source path segment exceeds 255 bytes or 255 UTF-16 code units",
        ),
        SkillImportError::PathTooLong => {
            bad_request("path_too_long", "a source path exceeds 1024 bytes")
        }
        SkillImportError::PathTooDeep => {
            bad_request("path_too_deep", "a source path exceeds 32 directory levels")
        }
        SkillImportError::UnsafePath => {
            bad_request("path_unsafe", "a source path is unsafe and was rejected")
        }
        SkillImportError::ArchiveExpansionRatioExceeded => bad_request(
            "archive_expansion_ratio_exceeded",
            "archive expands beyond the allowed ratio",
        ),
        SkillImportError::TotalBytesExceeded => bad_request(
            "archive_total_bytes_exceeded",
            "source exceeds the allowed cumulative byte budget",
        ),
        SkillImportError::TooManyEntries { max_entries } => bad_request(
            "too_many_entries",
            format!("source contains more than {max_entries} entries"),
        ),
        SkillImportError::PreparationTimeout => bad_request(
            "import_preparation_timeout",
            "import preparation exceeded the allowed time limit",
        ),
        SkillImportError::SessionNotFound { .. } => BackendError::new(
            BackendErrorKind::NotFound,
            "import_session_not_found",
            "import session not found",
        ),
        SkillImportError::SessionExpired => BackendError::new(
            BackendErrorKind::NotFound,
            "import_session_expired",
            "import session has expired",
        ),
        SkillImportError::SessionCancelled => BackendError::new(
            BackendErrorKind::Conflict,
            "import_session_cancelled",
            "import session was cancelled",
        ),
        SkillImportError::CommitInProgress => BackendError::new(
            BackendErrorKind::Conflict,
            "import_session_commit_in_progress",
            "import session commit is already in progress",
        ),
        SkillImportError::AlreadyCommitted => BackendError::new(
            BackendErrorKind::Conflict,
            "import_session_already_committed",
            "import session was already committed with different decisions",
        ),
        SkillImportError::DecisionMissing { .. } => bad_request(
            "import_decision_missing",
            "decisions are missing for some conflict candidates",
        ),
        SkillImportError::SourceUnavailable { .. } => internal(
            "import_source_unavailable",
            "the import source could not be read",
        ),
        SkillImportError::Storage { .. } => internal(
            "skill_storage_error",
            "skill storage operation failed during import",
        ),
        SkillImportError::Repository { .. } => internal(
            "skill_repository_error",
            "skill repository operation failed during import",
        ),
        SkillImportError::Internal { .. } => {
            internal("skill_import_error", "internal import failure")
        }
    }
}

/// Builds a sanitized client-failure for import validation errors.
fn bad_request(code: &'static str, message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorKind::BadRequest, code, message)
}

#[cfg(test)]
mod tests {
    use super::{BackendError, BackendErrorKind};
    use ora_application::ApplicationError;

    #[test]
    fn exposes_non_git_worktree_roots_as_a_stable_bad_request() {
        let error = BackendError::from(ApplicationError::TaskWorktreeRequiresGitRepository);

        assert_eq!(error.kind(), BackendErrorKind::BadRequest);
        assert_eq!(error.code(), "worktree_requires_git_repository");
        assert_eq!(error.message(), "worktree mode requires a Git repository");
    }
}

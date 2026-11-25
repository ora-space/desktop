use super::ErrorClassification;
use ora_application::SkillImportError;
use ora_contracts::{EmptyErrorParams, PublicError};

pub(super) fn map_skill_import_error(
    error: &SkillImportError,
) -> (ErrorClassification, PublicError, &'static str) {
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
        SkillImportError::TooManyFiles { .. } | SkillImportError::TooManyEntries { .. } => (
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
        SkillImportError::SessionNotFound { .. } | SkillImportError::SessionExpired => (
            ErrorClassification::NotFound,
            PublicError::ImportSessionExpired(params),
            "skill import session is unavailable",
        ),
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

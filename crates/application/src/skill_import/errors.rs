use ora_skill_package::PrepareError;
use thiserror::Error;

/// Reports one import-session failure that adapters translate into a stable public code.
///
/// Archive and path-safety errors carry no attacker-controlled raw paths, and per-candidate
/// manifest problems are surfaced as candidate error codes rather than session failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillImportError {
    #[error("no SKILL.md manifest was found in the source")]
    SkillManifestNotFound,
    #[error("source contains more than {max_skills} skills")]
    TooManySkills { max_skills: usize },
    #[error("one skill contains more than {max_files} files")]
    TooManyFiles { max_files: usize },
    #[error("multiple valid skills in one source declare the same name")]
    DuplicateSkillNames { duplicates: Vec<DuplicateSkillName> },
    #[error("unsupported archive format; allowed extensions: zip, skill, tar.gz, tgz")]
    ArchiveFormatUnsupported,
    #[error("archive contents do not match the requested format")]
    ArchiveFormatMismatch,
    #[error("archive is corrupt or unreadable")]
    ArchiveCorrupt,
    #[error("archive exceeds the maximum upload size")]
    ArchiveTooLarge,
    #[error("encrypted archives are not supported")]
    ArchiveEncryptedUnsupported,
    #[error("archive contains a special entry that cannot be stored safely")]
    ArchiveSpecialEntryUnsupported,
    #[error("archive entry path is not valid UTF-8")]
    ArchivePathEncodingInvalid,
    #[error("source paths conflict after portable case normalization")]
    ArchivePathCaseConflict,
    #[error("a source path segment exceeds 255 bytes or 255 UTF-16 code units")]
    PathSegmentTooLong,
    #[error("a source path exceeds 1024 bytes")]
    PathTooLong,
    #[error("a source path exceeds 32 directory levels")]
    PathTooDeep,
    #[error("a source path is unsafe and was rejected")]
    UnsafePath,
    #[error("archive expands beyond the allowed ratio")]
    ArchiveExpansionRatioExceeded,
    #[error("source exceeds the allowed cumulative byte budget")]
    TotalBytesExceeded,
    #[error("source contains more than {max_entries} entries")]
    TooManyEntries { max_entries: usize },
    #[error("import preparation exceeded the allowed time limit")]
    PreparationTimeout,
    #[error("import session not found: {session_id}")]
    SessionNotFound { session_id: String },
    #[error("import session has expired")]
    SessionExpired,
    #[error("import session was cancelled")]
    SessionCancelled,
    #[error("import session commit is already in progress")]
    CommitInProgress,
    #[error("import session was already committed with different decisions")]
    AlreadyCommitted,
    #[error("decisions are missing for conflict candidates: {candidate_ids:?}")]
    DecisionMissing { candidate_ids: Vec<String> },
    #[error("the import source could not be read: {message}")]
    SourceUnavailable { message: String },
    #[error("skill storage operation failed during import: {message}")]
    Storage { message: String },
    #[error("skill repository operation failed during import: {message}")]
    Repository { message: String },
    #[error("internal import failure: {message}")]
    Internal { message: String },
}

/// One duplicate skill name and the safe source paths that declare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateSkillName {
    pub name: String,
    pub source_paths: Vec<String>,
}

impl From<PrepareError> for SkillImportError {
    /// Converts snapshot-level source failures into stable import errors.
    fn from(error: PrepareError) -> Self {
        match error {
            PrepareError::ArchiveFormatUnsupported => Self::ArchiveFormatUnsupported,
            PrepareError::ArchiveFormatMismatch => Self::ArchiveFormatMismatch,
            PrepareError::ArchiveCorrupt => Self::ArchiveCorrupt,
            PrepareError::ArchiveTooLarge => Self::ArchiveTooLarge,
            PrepareError::ArchiveEncryptedUnsupported => Self::ArchiveEncryptedUnsupported,
            PrepareError::ArchiveSpecialEntryUnsupported => Self::ArchiveSpecialEntryUnsupported,
            PrepareError::ArchivePathEncodingInvalid => Self::ArchivePathEncodingInvalid,
            PrepareError::ArchivePathCaseConflict => Self::ArchivePathCaseConflict,
            PrepareError::PathSegmentTooLong => Self::PathSegmentTooLong,
            PrepareError::PathTooLong => Self::PathTooLong,
            PrepareError::PathTooDeep => Self::PathTooDeep,
            PrepareError::UnsafePath => Self::UnsafePath,
            PrepareError::ArchiveExpansionRatioExceeded => Self::ArchiveExpansionRatioExceeded,
            PrepareError::TotalBytesExceeded => Self::TotalBytesExceeded,
            PrepareError::TooManyEntries { max_entries } => Self::TooManyEntries { max_entries },
            PrepareError::Io { message } => Self::Internal { message },
        }
    }
}

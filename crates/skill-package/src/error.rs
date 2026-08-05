use thiserror::Error;

/// Reports failures that reject the entire source snapshot before any skill is committed.
///
/// Every variant maps to a stable public error code. Path-tampering variants deliberately
/// carry no raw attacker-controlled path so hostile names never reach the user or logs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PrepareError {
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
    #[error("failed to read or write the source snapshot: {message}")]
    Io { message: String },
}

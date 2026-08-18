use crate::path::StrictRelativePathError;
use thiserror::Error;

/// Reports failures that reject an entire archive or folder tree before it is used.
///
/// Every variant is safe to display: path-tampering variants deliberately carry no raw
/// attacker-controlled path so hostile names never reach the user or logs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArchiveError {
    #[error("archive contents do not match the requested format")]
    FormatMismatch,
    #[error("archive is corrupt or unreadable")]
    Corrupt,
    #[error("archive exceeds the maximum size")]
    TooLarge,
    #[error("encrypted archives are not supported")]
    EncryptedUnsupported,
    #[error("archive contains a special entry that cannot be stored safely")]
    SpecialEntryUnsupported,
    #[error("entry path is not valid UTF-8")]
    PathEncodingInvalid,
    #[error("entry paths conflict after portable case normalization")]
    PathCaseConflict,
    #[error("entry path was rejected: {0:?}")]
    Path(StrictRelativePathError),
    #[error("archive expands beyond the allowed ratio")]
    ExpansionRatioExceeded,
    #[error("tree exceeds the allowed cumulative byte budget")]
    TotalBytesExceeded,
    #[error("tree contains more than {max_entries} entries")]
    TooManyEntries { max_entries: usize },
    #[error("failed to read or write the tree: {message}")]
    Io { message: String },
}

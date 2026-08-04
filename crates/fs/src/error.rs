use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Describes workspace-scoped filesystem failures without leaking transport concerns into the crate.
#[derive(Debug, Error)]
pub enum WorkspaceFileSystemError {
    #[error("workspace root is unavailable: {path:?}")]
    WorkspaceUnavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workspace path must be relative: {path:?}")]
    PathNotRelative { path: PathBuf },
    #[error("workspace path escapes its root: {path:?}")]
    PathOutsideWorkspace { path: PathBuf },
    #[error("workspace path was not found: {path:?}")]
    PathNotFound { path: PathBuf },
    #[error("workspace path is not a directory: {path:?}")]
    NotDirectory { path: PathBuf },
    #[error("workspace path is not a file: {path:?}")]
    NotFile { path: PathBuf },
    #[error("workspace filesystem operation failed for {path:?}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workspace file is larger than {limit_bytes} bytes: {path:?}")]
    FileTooLarge { path: PathBuf, limit_bytes: u64 },
    #[error("workspace file is binary: {path:?}")]
    BinaryFile { path: PathBuf },
    #[error("workspace file is not valid UTF-8: {path:?}")]
    InvalidUtf8 { path: PathBuf },
    #[error("ripgrep is unavailable: {path:?}")]
    SearchToolUnavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("ripgrep search timed out")]
    SearchTimedOut,
    #[error("ripgrep output exceeded {limit_bytes} bytes")]
    SearchOutputTooLarge { limit_bytes: usize },
    #[error("ripgrep search failed: {message}")]
    SearchFailed { message: String },
    #[error("ripgrep returned malformed JSON")]
    InvalidSearchOutput {
        #[source]
        source: serde_json::Error,
    },
    #[error("workspace watcher failed for {path:?}: {message}")]
    WatchFailed { path: PathBuf, message: String },
}

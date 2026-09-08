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
    #[error("workspace path is not accessible: {path:?}")]
    PermissionDenied {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
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

impl WorkspaceFileSystemError {
    /// Separates permission denials from opaque I/O faults for one path-scoped operation.
    ///
    /// A denial is a user-visible access condition rather than a backend fault, so it must stay
    /// distinguishable from `Io` all the way to the transport; collapsing both into `Io` would
    /// report a routine access refusal as an internal error.
    pub(crate) fn from_path_io(path: PathBuf, source: io::Error) -> Self {
        if source.kind() == io::ErrorKind::PermissionDenied {
            Self::PermissionDenied { path, source }
        } else {
            Self::Io { path, source }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// A denial must stay distinguishable so the transport can classify it as an access refusal.
    #[test]
    fn classifies_permission_denials_apart_from_other_io_faults() {
        let path = PathBuf::from("workspace/secret");

        let denied = WorkspaceFileSystemError::from_path_io(
            path.clone(),
            io::Error::from(io::ErrorKind::PermissionDenied),
        );
        let other = WorkspaceFileSystemError::from_path_io(
            path,
            io::Error::from(io::ErrorKind::NotADirectory),
        );

        assert!(matches!(
            denied,
            WorkspaceFileSystemError::PermissionDenied { .. }
        ));
        assert!(matches!(other, WorkspaceFileSystemError::Io { .. }));
    }

    /// The denial must keep its cause so operators still see the originating I/O error.
    #[test]
    fn retains_the_denial_source() {
        let error = WorkspaceFileSystemError::from_path_io(
            PathBuf::from("workspace/secret"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        );

        let WorkspaceFileSystemError::PermissionDenied { path, source } = error else {
            panic!("expected a permission denial");
        };
        assert_eq!(
            (path, source.kind()),
            (
                PathBuf::from("workspace/secret"),
                io::ErrorKind::PermissionDenied
            )
        );
    }
}

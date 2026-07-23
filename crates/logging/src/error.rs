use std::path::PathBuf;

use thiserror::Error;

/// Identifies the filesystem step that failed while preparing file-backed logging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSystemAction {
    CreateDirectory,
    ReadDirectory,
    RemoveFile,
}

/// Describes the typed failures that can prevent shared logging from starting.
#[derive(Debug, Error)]
pub enum LoggingInitError {
    #[error("log file path must include a file name: {path}")]
    InvalidFilePath { path: PathBuf },
    #[error("failed to {action:?} at {path}")]
    FileSystem {
        action: FileSystemAction,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to install the global tracing subscriber")]
    SetGlobalSubscriber(#[source] tracing::dispatcher::SetGlobalDefaultError),
    #[error("the process-wide logging clock was already initialized")]
    ClockAlreadyInitialized,
}

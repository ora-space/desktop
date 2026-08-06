use std::path::PathBuf;
use thiserror::Error;

/// Reports why a session history file could not be written or read.
///
/// Callers are expected to treat every variant as a reason to stop writing that
/// session rather than to retry silently: a history file that skips records is
/// more dangerous than one that stops, because the gap is invisible to whoever
/// replays it later.
#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("failed to create history directory {path:?}")]
    DirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open history file {path:?}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to append to history file {path:?}")]
    Append {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read history file {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to remove history file {path:?}")]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode history record")]
    Encode(#[source] serde_json::Error),
    #[error("session identifier is not usable as a history file name: {session_id:?}")]
    InvalidSessionId { session_id: String },
}

use thiserror::Error;

/// Preserves filesystem, journal and identity failures without reconstructing missing responsibility.
#[derive(Debug, Error)]
pub enum ProcessStateError {
    #[error("process state filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("process state journal operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("process state rejected: {0}")]
    Rejected(&'static str),
    #[error("process state contains an invalid identity: {0}")]
    Identity(#[from] ora_process_protocol::InvalidProcessIdentity),
}

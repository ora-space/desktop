use thiserror::Error;

/// Reports framing, correlation, serialization, and transport failures.
#[derive(Debug, Error)]
pub enum AcpError {
    #[error("ACP stream ended before the pending operation completed")]
    StreamClosed,
    #[error("ACP transport I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("ACP frame is invalid: {0}")]
    InvalidFrame(String),
    #[error("ACP returned an operation error: {0}")]
    RequestFailed(String),
    #[error("ACP response payload is invalid: {0}")]
    InvalidResponse(String),
}

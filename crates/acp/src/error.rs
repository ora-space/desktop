use thiserror::Error;

/// An ACP request error as the provider returned it, before Ora classifies it.
///
/// The provider writes the message and optional `data`, so they are untrusted and must be
/// sanitized before they can be shown; the integer `code` is the JSON-RPC error code and is the
/// one ACP defines for authentication (`-32000`) and cancellation (`-32800`).
#[derive(Debug, Clone)]
pub struct AcpRequestError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for AcpRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

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
    RequestFailed(AcpRequestError),
    #[error("ACP response payload is invalid: {0}")]
    InvalidResponse(String),
}

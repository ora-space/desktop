//! Streaming progress reporting shared by download backends.

/// A snapshot of transfer progress: bytes completed and an optional known total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Bytes transferred so far.
    pub bytes: u64,
    /// Total expected bytes when the server reports a length; `None` when unknown.
    pub total: Option<u64>,
}

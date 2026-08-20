//! Structured, display-safe error type shared by every download backend.

use std::io;
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

/// Which phase of a transfer exceeded its budget. Only produced by network backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutPhase {
    /// The TCP/TLS connection could not be established in time.
    Connect,
    /// A single request attempt exceeded its per-attempt budget.
    PerAttempt,
    /// The whole download (including retries) exceeded its total budget.
    Total,
}

/// Reports why a download did not complete.
///
/// Every variant carries enough context (url, path, status) for an orchestrator to log without
/// post-processing, and no message leaks credentials. Network-only variants are produced by the
/// `ReqwestDownloader` backend, not by [`LocalFileDownloader`](crate::http::local::LocalFileDownloader).
#[derive(Debug, Error)]
pub enum DownloadError {
    /// A connection-level failure while talking to `url`.
    #[error("network error downloading {url}: {source}")]
    Network { url: Url, source: io::Error },
    /// The remote `url` answered with an unexpected HTTP status.
    #[error("download of {url} returned HTTP status {status}")]
    HttpStatus { url: Url, status: u16 },
    /// A local file could not be read or written at `path`.
    #[error("failed to read or write {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    /// The computed digest did not match the expected value.
    #[error("checksum mismatch for {url} (expected {expected:02x?}, got {actual:02x?})")]
    ChecksumMismatch {
        url: Url,
        expected: Vec<u8>,
        actual: Vec<u8>,
    },
    /// The transfer would exceed `limit` bytes.
    #[error("download of {url} exceeds the {limit} byte limit")]
    TooLarge { url: Url, limit: u64 },
    /// A transfer phase exceeded its time budget.
    #[error("download timed out during {phase:?}")]
    Timeout { phase: TimeoutPhase },
    /// The transfer was cancelled by the caller before completion.
    #[error("download cancelled")]
    Cancelled,
    /// The source could not be turned into a usable download.
    #[error("invalid download source: {0}")]
    InvalidSource(String),
}

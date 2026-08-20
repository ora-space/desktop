//! The downloader contract and the request/response data types that describe one download.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

use super::cancel::CancelToken;
use super::error::DownloadError;
use super::progress::Progress;

/// A callback invoked with transfer progress during a download.
pub type ProgressCallback = Arc<dyn Fn(Progress) + Send + Sync>;

/// The uniform download interface that callers inject, replace, and test through.
///
/// Implementations may transfer over the network or read a local file; the trait only insists that
/// a download produces the requested artifact at `destination` or fails with a structured error.
/// Implementations are expected to write through a same-directory temporary file and only replace
/// the destination once every check (byte limit, checksum) has succeeded.
pub trait HttpDownload: Send + Sync + 'static {
    /// Downloads `request.source` to `request.destination`, returning the produced byte count and
    /// the computed SHA-256 digest. The returned future is driven by the caller's own runtime.
    #[allow(clippy::manual_async_fn)] // explicit `+ Send` lets async-run-time callers spawn the future
    fn download(
        &self,
        request: DownloadRequest,
    ) -> impl Future<Output = Result<DownloadOutcome, DownloadError>> + Send;
}

/// Where the artifact bytes come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadSource {
    /// A remote `http://` or `https://` URL.
    Url(Url),
    /// A local path (`file://` semantics without a URL).
    Local(PathBuf),
}

/// One self-contained download operation.
#[derive(Clone)]
pub struct DownloadRequest {
    /// The artifact to fetch.
    pub source: DownloadSource,
    /// The destination file; written via a `<destination>.tmp` sibling that is renamed over it.
    pub destination: PathBuf,
    /// An optional expected digest; when present the download fails on any mismatch.
    pub checksum: Option<Checksum>,
    /// Per-download tuning that falls back to the module defaults.
    pub options: DownloadOptions,
    /// Optional progress reporting; only network backends emit it as they stream.
    pub progress: Option<ProgressCallback>,
    /// Optional cooperative cancellation; only network backends poll it between chunks.
    pub cancel: Option<CancelToken>,
}

/// Per-download tuning. Field defaults live in [`Default`]; the network backend consumes the
/// timeout and retry fields while both backends honor `max_bytes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadOptions {
    /// Rejects downloads that would grow beyond this many bytes to bound disk usage (None = no bound).
    pub max_bytes: Option<u64>,
    /// The TCP/TLS connect timeout; `None` uses the reqwest default.
    pub connect_timeout: Option<Duration>,
    /// The budget for a single request attempt; `None` uses the reqwest default.
    pub per_attempt_timeout: Option<Duration>,
    /// The overall budget including retries; `None` means no total cap.
    pub total_timeout: Option<Duration>,
    /// How many transient failures are retried before giving up.
    pub max_retries: u32,
    /// Base delay before the first retry; each attempt doubles the previous delay.
    pub retry_base_delay: Duration,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            max_bytes: None,
            connect_timeout: None,
            per_attempt_timeout: None,
            total_timeout: None,
            max_retries: 3,
            retry_base_delay: Duration::from_millis(200),
        }
    }
}

/// The digest result of a successful download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadOutcome {
    /// The number of bytes written to the destination.
    pub bytes: u64,
    /// The SHA-256 digest of the written artifact, computed while copying.
    pub sha256: [u8; 32],
}

/// Identifies the digest algorithm used for a checksum. `#[non_exhaustive]` reserves room for
/// future algorithms such as SHA-512 without breaking existing callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HashAlgorithm {
    /// SHA-256, the only algorithm currently supported.
    Sha256,
}

/// An expected digest paired with its algorithm.
///
/// The digest is stored as raw bytes rather than a hex string so hex parsing never leaks into the
/// download layer; domain callers that read hex from a manifest parse it before building this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checksum {
    algorithm: HashAlgorithm,
    digest: Vec<u8>,
}

impl Checksum {
    /// Builds a SHA-256 checksum from the raw digest bytes.
    pub fn sha256(digest: impl Into<Vec<u8>>) -> Self {
        Self {
            algorithm: HashAlgorithm::Sha256,
            digest: digest.into(),
        }
    }

    /// Returns the digest algorithm.
    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    /// Returns the raw digest bytes.
    pub fn digest(&self) -> &[u8] {
        &self.digest
    }
}

//! Single-request file download from a presigned URL, built on `reqwest`.
//!
//! The read counterpart of the presigned upload: a presigned URL is a bearer credential, so this
//! fetcher sends the caller's headers unchanged, never follows a redirect (which could forward the
//! signed headers to another origin), and never puts the URL into an error. It performs exactly
//! one request; a success body is streamed into a new owner-only file and measured on the way, so
//! the caller can verify size and digest before trusting the bytes. Whether a status means retry,
//! renew the grant or give up is the caller's policy.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use url::Url;

use super::proxy::ProxyConfig;
use super::reqwest::flatten_reqwest_error;
use super::upload::{SignedClientError, signed_client, signed_headers};

/// Bounds for one download request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchOptions {
    /// Budget for establishing the TCP/TLS connection.
    pub connect_timeout: Duration,
    /// Budget for the whole request, including streaming the body to the file.
    pub total_timeout: Duration,
}

/// One object to read with a presigned `GET`.
#[derive(Clone, Copy, Debug)]
pub struct FileFetch<'a> {
    /// Absolute HTTP(S) URL; treated as a secret and never included in errors.
    pub url: &'a str,
    /// Headers sent exactly as given.
    pub headers: &'a BTreeMap<String, String>,
    /// A path that must not exist yet; it is created owner-read/write only.
    pub destination: &'a Path,
    /// The body may not exceed this many bytes; a larger body is refused while streaming.
    pub max_bytes: u64,
}

/// What the single request produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    /// A success status; the body is in the destination with this size and SHA-256.
    Stored { bytes: u64, sha256: [u8; 32] },
    /// Any other status, redirects included; nothing was written.
    Status(u16),
}

/// Why no usable response was obtained. No variant carries the request URL, and nothing is left
/// at the destination.
#[derive(Debug, Error)]
pub enum FetchError {
    /// The request could not be built from the given URL or headers.
    #[error("invalid download request: {0}")]
    InvalidRequest(&'static str),
    /// The destination could not be created or written.
    #[error("failed to write {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    /// The connection failed before or while the body arrived.
    #[error("download network error: {0}")]
    Network(String),
    /// The request exceeded its time budget.
    #[error("download timed out")]
    Timeout,
    /// The body exceeded the caller's limit.
    #[error("download exceeds the {limit} byte limit")]
    TooLarge { limit: u64 },
}

/// Downloads objects with presigned requests over HTTP(S).
#[derive(Clone, Debug)]
pub struct ReqwestFetcher {
    proxy_config: ProxyConfig,
}

impl ReqwestFetcher {
    pub fn new(proxy_config: ProxyConfig) -> Self {
        Self { proxy_config }
    }

    /// Sends one `GET` and stores a success body; any partial file is removed on failure.
    pub async fn fetch(
        &self,
        fetch: FileFetch<'_>,
        options: FetchOptions,
    ) -> Result<FetchOutcome, FetchError> {
        let url = Url::parse(fetch.url).map_err(|_| FetchError::InvalidRequest("url"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchError::InvalidRequest("url scheme"));
        }
        let headers = signed_headers(fetch.headers).map_err(FetchError::InvalidRequest)?;
        let client = signed_client(
            &self.proxy_config,
            &url,
            options.connect_timeout,
            options.total_timeout,
        )
        .map_err(|error| match error {
            SignedClientError::Proxy => FetchError::InvalidRequest("proxy"),
            SignedClientError::Build(message) => FetchError::Network(message),
        })?;
        let transfer = async {
            let mut response = client
                .get(url)
                .headers(headers)
                .send()
                .await
                .map_err(network)?;
            if !response.status().is_success() {
                return Ok(FetchOutcome::Status(response.status().as_u16()));
            }
            let mut output =
                create_private(fetch.destination).map_err(|source| FetchError::Io {
                    path: fetch.destination.to_path_buf(),
                    source,
                })?;
            let written = async {
                let mut hasher = Sha256::new();
                let mut bytes: u64 = 0;
                while let Some(chunk) = response.chunk().await.map_err(network)? {
                    bytes += chunk.len() as u64;
                    if bytes > fetch.max_bytes {
                        return Err(FetchError::TooLarge {
                            limit: fetch.max_bytes,
                        });
                    }
                    hasher.update(&chunk);
                    output.write_all(&chunk).map_err(|source| FetchError::Io {
                        path: fetch.destination.to_path_buf(),
                        source,
                    })?;
                }
                output.sync_all().map_err(|source| FetchError::Io {
                    path: fetch.destination.to_path_buf(),
                    source,
                })?;
                Ok(FetchOutcome::Stored {
                    bytes,
                    sha256: hasher.finalize().into(),
                })
            }
            .await;
            if written.is_err() {
                let _ = std::fs::remove_file(fetch.destination);
            }
            written
        };
        match tokio::time::timeout(options.total_timeout, transfer).await {
            Ok(result) => result,
            Err(_elapsed) => {
                // The body may have been cut off mid-stream; a partial file is never kept.
                let _ = std::fs::remove_file(fetch.destination);
                Err(FetchError::Timeout)
            }
        }
    }
}

/// Classifies a transport failure without the URL reqwest would otherwise attach.
fn network(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        FetchError::Timeout
    } else {
        FetchError::Network(flatten_reqwest_error(&error.without_url()))
    }
}

/// Creates the destination exclusively, so an existing file or a planted link is never written
/// through, and owner-only, because the bytes were fetched with a credential.
fn create_private(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options
        .write(/*write*/ true)
        .create_new(/*create_new*/ true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, /*mode*/ 0o600);
    options.open(path)
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;

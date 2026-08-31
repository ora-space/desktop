//! HTTP(S) download backend built on `reqwest`.
//!
//! This backend streams a remote response straight to a verified destination while honoring byte
//! limits, per-phase timeouts, retries with backoff, progress callbacks, and cancellation. It talks
//! to the outside world through the injected `ProxyConfig` and the standard proxy environment
//! variables, so it needs an async runtime provided by the caller.

use sha2::{Digest, Sha256};
use std::error::Error as StdError;
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

use super::error::{DownloadError, TimeoutPhase};
use super::progress::Progress;
use super::proxy::{Proxy, ProxyConfig, resolve_proxy};
use super::target::{io_error, remove_temporary, rename_over, temporary_sibling};
use super::types::{
    DownloadOptions, DownloadOutcome, DownloadRequest, DownloadSource, HttpDownload,
};

/// User-Agent sent unless the caller overrides it on the downloader.
const DEFAULT_USER_AGENT: &str = "Ora-Desktop";
/// FNV prime and offset used to derive a deterministic per-attempt jitter without a RNG dependency.
const FNV_PRIME: u64 = 0x100000001b3;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;

/// Fetches remote release artifacts over HTTP(S) with progress, retries, and cancellation.
///
/// Construct one per stable proxy/User-Agent configuration and reuse it; each request still rebuilds
/// the underlying client so a proxy change is picked up without restarting.
#[derive(Clone, Debug)]
pub struct ReqwestDownloader {
    proxy_config: ProxyConfig,
    user_agent: String,
    tls_extra_roots: Vec<rustls::pki_types::CertificateDer<'static>>,
}

impl ReqwestDownloader {
    /// Creates a downloader using `proxy_config` and a default User-Agent.
    pub fn new(proxy_config: ProxyConfig) -> Self {
        Self {
            proxy_config,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            tls_extra_roots: Vec::new(),
        }
    }

    /// Adds a private CA root to explicit test trust for deterministic HTTPS regression tests.
    #[cfg(test)]
    pub(crate) fn with_extra_tls_root(
        mut self,
        root: rustls::pki_types::CertificateDer<'static>,
    ) -> Self {
        self.tls_extra_roots.push(root);
        self
    }

    /// Builds a downloader with an explicit User-Agent, useful for gateway/CDN identification.
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Runs one download, retrying transient failures with exponential backoff and jitter.
    async fn download_inner(
        &self,
        request: DownloadRequest,
    ) -> Result<DownloadOutcome, DownloadError> {
        let url = match &request.source {
            DownloadSource::Url(url) => url.clone(),
            DownloadSource::Local(_) => {
                return Err(DownloadError::InvalidSource(
                    "local sources require the local downloader".to_owned(),
                ));
            }
        };

        let operation = async { self.attempt_with_retries(&request, &url).await };
        match request.options.total_timeout {
            Some(total) => tokio::time::timeout(total, operation).await.map_err(|_| {
                DownloadError::Timeout {
                    phase: TimeoutPhase::Total,
                }
            })?,
            None => operation.await,
        }
    }

    /// Runs the per-attempt loop, sleeping and retrying only when the failure looks transient.
    async fn attempt_with_retries(
        &self,
        request: &DownloadRequest,
        url: &Url,
    ) -> Result<DownloadOutcome, DownloadError> {
        let mut attempts: u32 = 0;
        loop {
            if attempts > 0 {
                tokio::time::sleep(retry_delay(
                    request.options.retry_base_delay,
                    attempts,
                    url.as_str(),
                ))
                .await;
            }
            match self.single_attempt(request, url).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if is_retryable(&error) && attempts < request.options.max_retries => {
                    attempts += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Executes one non-retrying download attempt against `url`.
    async fn single_attempt(
        &self,
        request: &DownloadRequest,
        url: &Url,
    ) -> Result<DownloadOutcome, DownloadError> {
        let client = self.build_client(&request.options, url)?;
        let temporary = temporary_sibling(&request.destination);
        if let Some(parent) = request.destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| io_error(&request.destination, error))?;
        }

        let mut response = client.get(url.clone()).send().await.map_err(|error| {
            remove_temporary(&temporary);
            network_error(url, error)
        })?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            remove_temporary(&temporary);
            return Err(DownloadError::HttpStatus {
                url: url.clone(),
                status,
            });
        }
        let total = response.content_length();
        let mut output = File::create(&temporary).map_err(|error| {
            remove_temporary(&temporary);
            io_error(&temporary, error)
        })?;
        let mut hasher = Sha256::new();
        let mut transferred: u64 = 0;

        loop {
            if let Some(token) = &request.cancel
                && token.is_cancelled()
            {
                remove_temporary(&temporary);
                return Err(DownloadError::Cancelled);
            }
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    remove_temporary(&temporary);
                    return Err(network_error(url, error));
                }
            };
            if let Some(limit) = request.options.max_bytes
                && transferred + chunk.len() as u64 > limit
            {
                remove_temporary(&temporary);
                return Err(DownloadError::TooLarge {
                    url: url.clone(),
                    limit,
                });
            }
            output.write_all(&chunk).map_err(|error| {
                remove_temporary(&temporary);
                io_error(&temporary, error)
            })?;
            hasher.update(&chunk);
            transferred += chunk.len() as u64;
            if let Some(callback) = &request.progress {
                callback(Progress {
                    bytes: transferred,
                    total,
                });
            }
        }
        output.flush().map_err(|error| {
            remove_temporary(&temporary);
            io_error(&temporary, error)
        })?;
        output.sync_all().map_err(|error| {
            remove_temporary(&temporary);
            io_error(&temporary, error)
        })?;

        let digest: [u8; 32] = hasher.finalize().into();
        if let Some(checksum) = &request.checksum
            && checksum.digest() != digest.as_slice()
        {
            remove_temporary(&temporary);
            return Err(DownloadError::ChecksumMismatch {
                url: url.clone(),
                expected: checksum.digest().to_vec(),
                actual: digest.to_vec(),
            });
        }
        rename_over(&request.destination, &temporary)?;
        Ok(DownloadOutcome {
            bytes: transferred,
            sha256: digest,
        })
    }

    /// Builds a per-request client with the resolved proxy and the requested timeouts.
    fn build_client(
        &self,
        options: &DownloadOptions,
        url: &Url,
    ) -> Result<reqwest::Client, DownloadError> {
        let tls_config =
            platform_tls_config(&self.tls_extra_roots).map_err(|error| DownloadError::Network {
                url: url.clone(),
                source: std::io::Error::other(format!(
                    "failed to initialize platform TLS verification: {error}"
                )),
            })?;
        let mut builder = reqwest::Client::builder()
            .use_preconfigured_tls(tls_config)
            .user_agent(&self.user_agent);
        if let Some(connect) = options.connect_timeout {
            builder = builder.connect_timeout(connect);
        }
        if let Some(attempt) = options.per_attempt_timeout {
            builder = builder.timeout(attempt);
        }
        if let Some(proxy) = resolve_proxy(url, &self.proxy_config) {
            builder = builder.proxy(proxy_reqwest(proxy)?);
        }
        builder.build().map_err(|error| network_error(url, error))
    }
}

/// Builds a Rustls client with certificate verification adapted to the host platform.
///
/// Copying native roots into a WebPKI store is insufficient on Windows because it bypasses the
/// CryptoAPI chain engine, including enterprise policy and intermediate discovery. The explicit
/// roots branch is test-only so HTTPS behavior can be exercised without modifying machine trust.
fn platform_tls_config(
    extra_roots: &[rustls::pki_types::CertificateDer<'static>],
) -> Result<rustls::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier: Arc<dyn rustls::client::danger::ServerCertVerifier> = if extra_roots.is_empty() {
        Arc::new(
            rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
                .map_err(|error| error.to_string())?,
        )
    } else {
        let mut roots = rustls::RootCertStore::empty();
        for root in extra_roots.iter().cloned() {
            roots.add(root).map_err(|error| error.to_string())?;
        }
        rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::clone(&provider),
        )
        .build()
        .map_err(|error| error.to_string())?
    };
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

impl HttpDownload for ReqwestDownloader {
    #[allow(clippy::manual_async_fn)] // match the trait's explicit `+ Send` future bound
    fn download(
        &self,
        request: DownloadRequest,
    ) -> impl Future<Output = Result<DownloadOutcome, DownloadError>> + Send {
        async move { self.download_inner(request).await }
    }
}

/// Converts a resolved `Proxy` into a reqwest proxy, applying any credentials.
fn proxy_reqwest(proxy: Proxy) -> Result<reqwest::Proxy, DownloadError> {
    let mut reqwest_proxy = reqwest::Proxy::all(proxy.endpoint.as_str())
        .map_err(|error| DownloadError::InvalidSource(error.to_string()))?;
    if let Some(auth) = proxy.auth {
        reqwest_proxy = reqwest_proxy.basic_auth(&auth.username, &auth.password);
    }
    Ok(reqwest_proxy)
}

/// Wraps a reqwest transport error as a structured error, carrying a URL for context.
///
/// The shared error model exposes `io::Error` as the transport source, so the richer `reqwest::Error`
/// is flattened into a displayable error string that preserves the full cause chain, so the real
/// failure reason (for example a TLS certificate verification error) is not hidden behind the
/// outermost "error sending request" message.
fn network_error(url: &Url, error: reqwest::Error) -> DownloadError {
    DownloadError::Network {
        url: url.clone(),
        source: std::io::Error::other(flatten_reqwest_error(&error)),
    }
}

/// Flattens a reqwest error and its whole cause chain into a single displayable string.
///
/// reqwest's `Display` only surfaces the outermost error (for example "error sending request for
/// url ..."), hiding the underlying cause (such as "invalid peer certificate: UnknownIssuer") in
/// its `source()` chain. Joining every link with " <- " keeps that context without pulling in a
/// logging dependency.
fn flatten_reqwest_error(error: &reqwest::Error) -> String {
    let mut links: Vec<String> = vec![error.to_string()];
    let mut current: Option<&(dyn StdError + 'static)> = error.source();
    while let Some(cause) = current {
        links.push(cause.to_string());
        current = cause.source();
    }
    links.join(" <- ")
}

/// Returns true for failures that are worth retrying (transient or rate-limited).
fn is_retryable(error: &DownloadError) -> bool {
    match error {
        DownloadError::Network { .. } | DownloadError::Timeout { .. } => true,
        DownloadError::HttpStatus { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

/// Computes the delay before retry `attempt`:  exponential backoff with a small jitter.
fn retry_delay(base: Duration, attempt: u32, seed: &str) -> Duration {
    let exponent = base.saturating_mul(1_u32 << attempt.min(10));
    let jitter = 75 + jitter_percent(seed, attempt);
    exponent.saturating_mul(jitter) / 100
}

/// Returns a deterministic 0..100 value from a seed and attempt count, used as jitter.
fn jitter_percent(seed: &str, attempt: u32) -> u32 {
    let mut hash = FNV_OFFSET;
    for byte in seed.bytes().chain([attempt as u8]) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    (hash % 100) as u32
}

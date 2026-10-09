//! Single-request file upload to a presigned URL, built on `reqwest`.
//!
//! A presigned upload is a bearer credential bound to exact headers, so this uploader sends the
//! caller's headers unchanged, never follows a redirect (which could forward the body and signed
//! headers to another origin), and never puts the URL into an error. It performs exactly one
//! request: whether a status means success, "already there" or "retry" is the caller's policy.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use url::Url;

use super::proxy::{ProxyConfig, resolve_proxy};
use super::reqwest::{flatten_reqwest_error, platform_tls_config, proxy_reqwest};

/// Bounds for one upload request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadOptions {
    /// Budget for establishing the TCP/TLS connection.
    pub connect_timeout: Duration,
    /// Budget for the whole request, including streaming the body and reading the status.
    pub total_timeout: Duration,
}

/// One file to send with a presigned request.
#[derive(Clone, Copy, Debug)]
pub struct FileUpload<'a> {
    /// HTTP method the URL was signed for, such as `PUT`.
    pub method: &'a str,
    /// Absolute HTTP(S) URL; treated as a secret and never included in errors.
    pub url: &'a str,
    /// Headers sent exactly as given; `Content-Length` is added only when absent.
    pub headers: &'a BTreeMap<String, String>,
    /// File streamed as the request body.
    pub file: &'a Path,
}

/// Why no HTTP status was obtained. No variant carries the request URL.
#[derive(Debug, Error)]
pub enum UploadError {
    /// The request could not be built from the given method, URL or headers.
    #[error("invalid upload request: {0}")]
    InvalidRequest(&'static str),
    /// The local file could not be opened or measured.
    #[error("failed to read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    /// The connection failed before a response status arrived.
    #[error("upload network error: {0}")]
    Network(String),
    /// The request exceeded its time budget.
    #[error("upload timed out")]
    Timeout,
}

/// Uploads files with presigned requests over HTTP(S).
#[derive(Clone, Debug)]
pub struct ReqwestUploader {
    proxy_config: ProxyConfig,
}

impl ReqwestUploader {
    pub fn new(proxy_config: ProxyConfig) -> Self {
        Self { proxy_config }
    }

    /// Sends the file once and returns the response status, whatever it is.
    pub async fn send(
        &self,
        upload: FileUpload<'_>,
        options: UploadOptions,
    ) -> Result<u16, UploadError> {
        let url = Url::parse(upload.url).map_err(|_| UploadError::InvalidRequest("url"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(UploadError::InvalidRequest("url scheme"));
        }
        let method = reqwest::Method::from_bytes(upload.method.as_bytes())
            .map_err(|_| UploadError::InvalidRequest("method"))?;
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in upload.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| UploadError::InvalidRequest("header name"))?;
            let value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| UploadError::InvalidRequest("header value"))?;
            headers.append(name, value);
        }
        let io_error = |source| UploadError::Io {
            path: upload.file.to_path_buf(),
            source,
        };
        let file = tokio::fs::File::open(upload.file).await.map_err(io_error)?;
        let length = file.metadata().await.map_err(io_error)?.len();
        // Object stores reject chunked uploads to presigned URLs, so the streamed body always
        // carries its exact length unless the signature already fixed one.
        if !headers.contains_key(reqwest::header::CONTENT_LENGTH) {
            headers.insert(reqwest::header::CONTENT_LENGTH, length.into());
        }
        let client = self.client(&url, options)?;
        let request = client
            .request(method, url)
            .headers(headers)
            .body(reqwest::Body::from(file));
        let response = tokio::time::timeout(options.total_timeout, request.send())
            .await
            .map_err(|_| UploadError::Timeout)?
            .map_err(|error| {
                if error.is_timeout() {
                    UploadError::Timeout
                } else {
                    UploadError::Network(flatten_reqwest_error(&error.without_url()))
                }
            })?;
        Ok(response.status().as_u16())
    }

    /// Builds a client that keeps the signed request on its origin and honors proxy settings.
    fn client(&self, url: &Url, options: UploadOptions) -> Result<reqwest::Client, UploadError> {
        let tls = platform_tls_config(&[]).map_err(UploadError::Network)?;
        let mut builder = reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(options.connect_timeout)
            .timeout(options.total_timeout);
        if let Some(proxy) = resolve_proxy(url, &self.proxy_config) {
            builder = builder
                .proxy(proxy_reqwest(proxy).map_err(|_| UploadError::InvalidRequest("proxy"))?);
        }
        builder
            .build()
            .map_err(|error| UploadError::Network(flatten_reqwest_error(&error.without_url())))
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;

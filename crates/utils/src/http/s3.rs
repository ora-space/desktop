//! S3-compatible download support: path-style object URLs, SigV4 signing, and a downloader that
//! signs object-key sources and path-style HTTPS URLs that target the configured bucket.

mod sign;

#[cfg(feature = "http-reqwest")]
mod downloader;

use std::fmt;
use url::Url;

pub use sign::{SigningTime, sign_get};

#[cfg(feature = "http-reqwest")]
pub use downloader::S3AwareDownloader;

use super::error::DownloadError;

/// Connection parameters for one S3-compatible endpoint.
///
/// Secrets are never written by [`Debug`]; callers must also keep them out of logs.
#[derive(Clone, Eq, PartialEq)]
pub struct S3Config {
    endpoint: String,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
}

impl S3Config {
    /// Builds a config from the endpoint host, bucket, region, and credentials.
    ///
    /// `endpoint` is a hostname (and optional port) without a scheme, for example
    /// `s3.example.com`.
    pub fn new(
        endpoint: impl Into<String>,
        bucket: impl Into<String>,
        region: impl Into<String>,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            region: region.into(),
            access_key: access_key.into(),
            secret_key: secret_key.into(),
        }
    }

    /// Returns the S3-compatible hostname (no scheme).
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Returns the bucket that object keys are resolved against.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Returns the SigV4 region name.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// Returns the access key id.
    pub fn access_key(&self) -> &str {
        &self.access_key
    }

    /// Returns the secret access key.
    pub fn secret_key(&self) -> &str {
        &self.secret_key
    }
}

impl fmt::Debug for S3Config {
    /// Redacts credentials so a dumped config cannot leak secrets.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key", &"[redacted]")
            .field("secret_key", &"[redacted]")
            .finish()
    }
}

/// Builds a path-style HTTPS URL `https://{endpoint}/{bucket}/{key}` without string-joining paths.
pub fn path_style_object_url(
    endpoint: &str,
    bucket: &str,
    key: &str,
) -> Result<Url, DownloadError> {
    if bucket.is_empty() {
        return Err(DownloadError::InvalidSource(
            "S3 bucket must not be empty".to_owned(),
        ));
    }
    if key.is_empty() {
        return Err(DownloadError::InvalidSource(
            "S3 object key must not be empty".to_owned(),
        ));
    }
    let mut url = Url::parse("https://placeholder.invalid").map_err(|error| {
        DownloadError::InvalidSource(format!("failed to parse S3 base URL: {error}"))
    })?;
    set_endpoint_host(&mut url, endpoint)?;
    {
        let mut segments = url.path_segments_mut().map_err(|()| {
            DownloadError::InvalidSource("S3 URL cannot accept path segments".to_owned())
        })?;
        segments.clear();
        segments.push(bucket);
        for segment in key.split('/') {
            segments.push(segment);
        }
    }
    Ok(url)
}

/// Returns the object key when `url` is a path-style HTTPS URL for `endpoint`/`bucket`.
///
/// Matches `https://{endpoint}/{bucket}/{key…}` (optional port on the endpoint). Other hosts,
/// buckets, schemes, or a URL that stops at the bucket root yield `None` so callers can fall
/// back to an unsigned download.
pub fn path_style_object_key(url: &Url, endpoint: &str, bucket: &str) -> Option<String> {
    if url.scheme() != "https"
        || bucket.is_empty()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let (expected_host, expected_port) = split_host_port(endpoint);
    if url.host_str() != Some(expected_host) {
        return None;
    }
    if !ports_match(url.port(), expected_port) {
        return None;
    }
    let mut segments = url.path_segments()?;
    let first = segments.next()?;
    if first != bucket {
        return None;
    }
    let key_segments: Vec<&str> = segments.filter(|segment| !segment.is_empty()).collect();
    if key_segments.is_empty() {
        return None;
    }
    // `Url` already collapses `.` / `..` path segments before we read them, so a listing cannot
    // smuggle a parent-directory hop past the bucket prefix without changing the first segment.
    Some(key_segments.join("/"))
}

/// Compares an optional URL port with an endpoint's optional port, treating the HTTPS default
/// as equivalent to an omitted port.
fn ports_match(url_port: Option<u16>, endpoint_port: Option<u16>) -> bool {
    match (url_port, endpoint_port) {
        (None, None) => true,
        (Some(actual), Some(expected)) => actual == expected,
        // `Url::port()` omits the scheme default (443 for HTTPS); an endpoint written without a
        // port means the same default.
        (None, Some(443)) | (Some(443), None) => true,
        _ => false,
    }
}

/// Applies `endpoint` as a hostname, optionally with a numeric port (`host:port`).
fn set_endpoint_host(url: &mut Url, endpoint: &str) -> Result<(), DownloadError> {
    let (host, port) = split_host_port(endpoint);
    url.set_host(Some(host)).map_err(|error| {
        DownloadError::InvalidSource(format!("invalid S3 endpoint host {host}: {error}"))
    })?;
    if let Some(port) = port
        && url.set_port(Some(port)).is_err()
    {
        return Err(DownloadError::InvalidSource(format!(
            "invalid S3 endpoint port {port}"
        )));
    }
    Ok(())
}

/// Splits `host` or `host:port` without treating the last colon of a non-numeric suffix as a port.
fn split_host_port(endpoint: &str) -> (&str, Option<u16>) {
    let Some((host, port)) = endpoint.rsplit_once(':') else {
        return (endpoint, None);
    };
    if host.is_empty() {
        return (endpoint, None);
    }
    match port.parse::<u16>() {
        Ok(port) => (host, Some(port)),
        Err(_) => (endpoint, None),
    }
}

#[cfg(test)]
mod tests {
    use super::{S3Config, path_style_object_key, path_style_object_url};
    use pretty_assertions::assert_eq;
    use url::Url;

    /// Path-style URLs join bucket and key as URL segments, preserving nested keys.
    #[test]
    fn builds_path_style_object_urls() {
        let url = path_style_object_url("s3.example.com", "my-bucket", "plugins/pkg.orax").unwrap();
        assert_eq!(
            url.as_str(),
            "https://s3.example.com/my-bucket/plugins/pkg.orax"
        );
    }

    /// An endpoint with a port is preserved so loopback tests can address a bound listener.
    #[test]
    fn builds_path_style_urls_with_a_port() {
        let url = path_style_object_url("localhost:8443", "bucket", "pkg.orax").unwrap();
        assert_eq!(url.as_str(), "https://localhost:8443/bucket/pkg.orax");
    }

    /// A marketplace-style path URL yields the key after the bucket segment.
    #[test]
    fn extracts_object_key_from_path_style_url() {
        let url = Url::parse(
            "https://s3.example.com/plugin-artifacts/agent/example.codeagent-v0.5.1.orax",
        )
        .unwrap();
        assert_eq!(
            path_style_object_key(&url, "s3.example.com", "plugin-artifacts").as_deref(),
            Some("agent/example.codeagent-v0.5.1.orax")
        );
    }

    /// Foreign hosts, wrong buckets, bare bucket URLs, and parent segments do not extract.
    #[test]
    fn rejects_non_matching_path_style_urls() {
        let foreign =
            Url::parse("https://github.com/org/repo/releases/download/v1/pkg.orax").unwrap();
        assert_eq!(
            path_style_object_key(&foreign, "s3.example.com", "bucket"),
            None
        );

        let wrong_bucket = Url::parse("https://s3.example.com/other-bucket/pkg.orax").unwrap();
        assert_eq!(
            path_style_object_key(&wrong_bucket, "s3.example.com", "bucket"),
            None
        );

        let bucket_only = Url::parse("https://s3.example.com/bucket").unwrap();
        assert_eq!(
            path_style_object_key(&bucket_only, "s3.example.com", "bucket"),
            None
        );
    }

    /// Debug output redacts both credential fields.
    #[test]
    fn debug_redacts_credentials() {
        let config = S3Config::new(
            "s3.example.com",
            "bucket",
            "us-east-1",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        );
        let rendered = format!("{config:?}");
        assert!(rendered.contains("[redacted]"));
        assert!(!rendered.contains("AKIDEXAMPLE"));
        assert!(!rendered.contains("wJalrXUtnFEMI"));
    }
}

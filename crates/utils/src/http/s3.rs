//! S3-compatible download support: path-style object URLs, SigV4 signing, and a downloader that
//! signs object-key sources while leaving HTTPS URLs unsigned.

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
    use super::{S3Config, path_style_object_url};
    use pretty_assertions::assert_eq;

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

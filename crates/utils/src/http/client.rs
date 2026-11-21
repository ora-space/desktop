//! A `reqwest` client for requests that carry a credential bound to one origin.
//!
//! Presigned URLs, signed headers and bearer tokens all share the same rule: the request must
//! reach exactly the origin it was built for. Following a redirect would forward the credential to
//! wherever the response points, so this client never follows one; the caller sees the redirect
//! status and decides. Certificates are verified with the platform-aware verifier, and the proxy is
//! resolved per URL from the caller's configuration.

use std::time::Duration;
use url::Url;

use super::proxy::{ProxyConfig, resolve_proxy};
use super::reqwest::{flatten_reqwest_error, platform_tls_config, proxy_reqwest};

/// Why a credential-carrying client could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialClientError {
    /// The configured proxy could not be applied.
    Proxy,
    /// TLS or client construction failed; the message never names the request URL.
    Build(String),
}

/// Builds a client that keeps every request on its origin and honors the proxy configuration.
///
/// `url` only selects the proxy; the client itself may send several requests to the same origin.
pub fn credential_client(
    proxy_config: &ProxyConfig,
    url: &Url,
    connect_timeout: Duration,
    total_timeout: Duration,
) -> Result<reqwest::Client, CredentialClientError> {
    let tls = platform_tls_config(&[]).map_err(CredentialClientError::Build)?;
    let mut builder = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(connect_timeout)
        .timeout(total_timeout);
    if let Some(proxy) = resolve_proxy(url, proxy_config) {
        builder = builder.proxy(proxy_reqwest(proxy).map_err(|_| CredentialClientError::Proxy)?);
    }
    builder
        .build()
        .map_err(|error| CredentialClientError::Build(flatten_reqwest_error(&error.without_url())))
}

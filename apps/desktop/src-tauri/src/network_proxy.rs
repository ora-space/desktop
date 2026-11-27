//! Shares Desktop HTTP proxy routing between the settings check and the signed updater.

use ora_application::NetworkProxyPreferences;
use ora_backend::Settings;
use ora_contracts::{CheckProxySettingsResponse, ProxyRoute};
use ora_utils::http::{HostProxy, HostProxyError, ProxyOrigin, is_proxy_auth_failure};
use std::time::Duration;
use updater_http::{ClientBuilder, Proxy, redirect::Policy};
use url::Url;

#[cfg(test)]
mod tests;

const PROXY_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Configures the same host proxy and redirect rules for every Desktop client used here.
pub(crate) fn configure_client(builder: ClientBuilder, policy: &HostProxy) -> ClientBuilder {
    let proxy = policy.clone();
    let redirect = policy.clone();
    let limit = Policy::limited(10);
    builder
        .no_proxy()
        .proxy(Proxy::custom(move |target| {
            proxy.route(target).ok()?.endpoint
        }))
        .redirect(Policy::custom(move |attempt| {
            let url = attempt.url();
            let downgrade = url.scheme() == "http"
                && attempt
                    .previous()
                    .last()
                    .is_some_and(|previous| previous.scheme() == "https");
            if !is_probe_url(url) || downgrade {
                return attempt.error("unsafe HTTP redirect");
            }
            if let Err(error) = redirect.route(url) {
                return attempt.error(error);
            }
            limit.redirect(attempt)
        }))
}

/// Probes a URL with the same transport as the updater, without saving the supplied preferences.
pub(crate) async fn check(
    preferences: &NetworkProxyPreferences,
    input: &str,
) -> CheckProxySettingsResponse {
    let unreachable = |message: String| CheckProxySettingsResponse::Unreachable { message };
    let Ok(url) = Url::parse(input) else {
        return unreachable("invalid probe URL".to_owned());
    };
    if !is_probe_url(&url) {
        return unreachable("URL must use HTTP(S) without credentials".to_owned());
    }
    let policy = match Settings::network_proxy_policy_for(preferences) {
        Ok(policy) => policy,
        Err(error) => return unreachable(error.to_string()),
    };
    match probe(updater_http::Client::builder(), &policy, url).await {
        Ok(result) => result,
        Err(message) => unreachable(message),
    }
}

/// Sends a bounded GET, reports the final route, and discards the response body.
async fn probe(
    builder: ClientBuilder,
    policy: &HostProxy,
    url: Url,
) -> Result<CheckProxySettingsResponse, String> {
    policy.route(&url).map_err(|error| error.to_string())?;
    // Reqwest uses the same ring provider here as the updater plugin does.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = configure_client(builder, policy)
        .timeout(PROXY_CHECK_TIMEOUT)
        .build()
        .map_err(probe_error)?;
    let response = client.get(url).send().await.map_err(probe_error)?;
    let status = response.status();
    if status.as_u16() == 407 {
        return Err(HostProxyError::Authentication.to_string());
    }
    if !status.is_success() {
        return Ok(CheckProxySettingsResponse::HttpError {
            status: status.as_u16(),
        });
    }
    let route = policy
        .route(response.url())
        .map_err(|error| error.to_string())?;
    Ok(CheckProxySettingsResponse::Reachable {
        status: status.as_u16(),
        route: Some(match route.origin {
            ProxyOrigin::Direct => ProxyRoute::Direct,
            ProxyOrigin::Manual => ProxyRoute::Manual,
            ProxyOrigin::System => ProxyRoute::System,
            ProxyOrigin::Environment => ProxyRoute::Environment,
        }),
    })
}

/// Keeps destination URLs and their query parameters out of probe errors.
fn probe_error(error: updater_http::Error) -> String {
    if is_proxy_auth_failure(&error) {
        HostProxyError::Authentication.to_string()
    } else {
        ora_utils::error::error_chain(&error.without_url())
    }
}

/// Validates the allowed URL schemes and rejects credentials at any hop.
fn is_probe_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
}

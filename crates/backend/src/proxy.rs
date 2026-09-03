use gitlancer::GitEnv;
use ora_application::NetworkProxySettings;
use ora_contracts::CheckProxySettingsResponse;
use ora_utils::http::{Proxy, ProxyAuth, ProxyBypass, ProxyConfig, ReqwestDownloader};
use std::time::Duration;
use url::Url;

use crate::BackendError;

/// Time budget for one proxy connectivity probe, including connect and the first response headers.
const PROXY_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the per-download proxy configuration selected by a marketplace source.
pub(crate) fn download_proxy(
    settings: Option<&NetworkProxySettings>,
) -> Result<Option<ProxyConfig>, BackendError> {
    let Some(settings) = settings else {
        return Ok(None);
    };
    let endpoint = proxy_endpoint(settings)?;
    let auth = match (&settings.username, &settings.password) {
        (Some(username), Some(password)) => Some(ProxyAuth {
            username: username.clone(),
            password: password.clone(),
        }),
        (Some(username), None) => Some(ProxyAuth {
            username: username.clone(),
            password: String::new(),
        }),
        (None, Some(password)) => Some(ProxyAuth {
            username: String::new(),
            password: password.clone(),
        }),
        (None, None) => None,
    };

    Ok(Some(ProxyConfig {
        explicit: Some(Proxy { endpoint, auth }),
        use_env: false,
        use_system: false,
        bypass: ProxyBypass::default(),
    }))
}

/// Builds the environment used by Git network commands when a source opts into proxying.
pub(crate) fn git_proxy_env(
    settings: Option<&NetworkProxySettings>,
) -> Result<Option<GitEnv>, BackendError> {
    let Some(settings) = settings else {
        return Ok(None);
    };
    let proxy_url = proxy_endpoint_with_credentials(settings)?;
    let proxy_url = proxy_url.as_str();

    let env = GitEnv::automation_defaults()
        .with_variable("http_proxy", proxy_url)
        .with_variable("HTTP_PROXY", proxy_url)
        .with_variable("https_proxy", proxy_url)
        .with_variable("HTTPS_PROXY", proxy_url)
        .with_variable("all_proxy", proxy_url)
        .with_variable("ALL_PROXY", proxy_url);

    Ok(Some(env))
}

/// Returns the plain endpoint URL from user-provided proxy settings.
fn proxy_endpoint(settings: &NetworkProxySettings) -> Result<Url, BackendError> {
    let base = normalized_proxy_url(settings)?;
    Url::parse(&base).map_err(|error| BackendError::invalid_proxy_settings(error.to_string()))
}

/// Returns a proxy URL with configured credentials embedded for Git's environment contract.
fn proxy_endpoint_with_credentials(settings: &NetworkProxySettings) -> Result<Url, BackendError> {
    let mut endpoint = proxy_endpoint(settings)?;
    if let Some(username) = &settings.username {
        endpoint.set_username(username).map_err(|()| {
            BackendError::invalid_proxy_settings(format!("invalid proxy username: {username}"))
        })?;
    }
    if let Some(password) = &settings.password {
        endpoint.set_password(Some(password)).map_err(|()| {
            BackendError::invalid_proxy_settings("invalid proxy password".to_string())
        })?;
    }
    Ok(endpoint)
}

/// Normalizes the configured host and port into a URL base, defaulting to `http://`.
fn normalized_proxy_url(settings: &NetworkProxySettings) -> Result<String, BackendError> {
    let host = settings.host.trim();
    if host.is_empty() {
        return Err(BackendError::invalid_proxy_settings(
            "proxy host must not be blank".to_string(),
        ));
    }
    if settings.port == 0 {
        return Err(BackendError::invalid_proxy_settings(
            "proxy port must be greater than zero".to_string(),
        ));
    }

    if host.contains("://") {
        Ok(format!("{host}:{}", settings.port))
    } else {
        Ok(format!("http://{host}:{}", settings.port))
    }
}

/// Probes `url` through `settings` and reports whether the proxy path reached a host.
///
/// Invalid URLs and transport failures are returned as `Unreachable` so the Settings UI can show a
/// check result instead of treating the probe as a command failure. Any HTTP status, including
/// 4xx and 5xx, means the proxy could talk to a remote host.
pub(crate) async fn check_proxy(
    settings: &NetworkProxySettings,
    url: &str,
) -> Result<CheckProxySettingsResponse, BackendError> {
    let parsed = match Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => parsed,
        Ok(_) => {
            return Ok(CheckProxySettingsResponse::Unreachable {
                message: "URL must use http or https".to_owned(),
            });
        }
        Err(error) => {
            return Ok(CheckProxySettingsResponse::Unreachable {
                message: error.to_string(),
            });
        }
    };
    let proxy_config = match download_proxy(Some(settings))? {
        Some(config) => config,
        None => {
            return Ok(CheckProxySettingsResponse::Unreachable {
                message: "proxy settings are incomplete".to_owned(),
            });
        }
    };
    match ReqwestDownloader::new(proxy_config)
        .probe(parsed, PROXY_CHECK_TIMEOUT)
        .await
    {
        Ok(status) => Ok(CheckProxySettingsResponse::Reachable { status }),
        Err(error) => Ok(CheckProxySettingsResponse::Unreachable {
            message: error.to_string(),
        }),
    }
}

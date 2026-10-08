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

/// Selects how Git reaches one remote, independent of the user's own Git proxy configuration.
pub(crate) enum GitProxyRoute<'a> {
    /// Connect without any proxy, even if Git config or the environment names one.
    Direct,
    /// Connect through the proxy Ora is configured with.
    Proxy(&'a NetworkProxySettings),
}

/// Builds the environment that pins Git's proxy for `remote_url` to `route`.
///
/// Git prefers `http.<url>.proxy` over `http.proxy`, and either over the `*_proxy` environment
/// variables, so the route is written as config for both the remote's origin and its full URL:
/// the origin entry outranks a user's host-wide entry and still applies when the checkout's
/// recorded remote is spelled differently, while the full-URL entry outranks a user's path-scoped
/// entry. An empty value disables proxying outright, which is what keeps a direct source direct.
pub(crate) fn git_proxy_env(
    remote_url: &str,
    route: GitProxyRoute<'_>,
) -> Result<GitEnv, BackendError> {
    let origin = Url::parse(remote_url)
        .map_err(|error| BackendError::internal("marketplace source URL is invalid", error))?
        .origin()
        .ascii_serialization();
    let proxy_url = match route {
        GitProxyRoute::Direct => String::new(),
        GitProxyRoute::Proxy(settings) => proxy_endpoint_with_credentials(settings)?.into(),
    };

    Ok(GitEnv::automation_defaults()
        .with_config(format!("http.{origin}.proxy"), proxy_url.as_str())
        .with_config(format!("http.{remote_url}.proxy"), proxy_url))
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

#[cfg(test)]
mod tests {
    use super::*;
    use gitlancer::{CliGitRunner, GitCommand, GitExecError, GitIntent, GitRunner};
    use pretty_assertions::assert_eq;
    use std::path::Path;

    /// Remote that never resolves, so every probe fails offline with a message naming its route.
    const REMOTE_URL: &str = "https://example.invalid/org/marketplace.git";

    /// User Git config that proxies the remote at every scope Git lets a user pick.
    const USER_PROXY_CONFIG: &str = "[http]\n\tproxy = http://user-proxy.invalid:1\n\
        [http \"https://example.invalid\"]\n\tproxy = http://user-proxy.invalid:1\n\
        [http \"https://example.invalid/org\"]\n\tproxy = http://user-proxy.invalid:1\n";

    /// Returns proxy settings that point at a host which never resolves.
    fn ora_proxy_settings() -> NetworkProxySettings {
        NetworkProxySettings {
            host: "ora-proxy.invalid".to_owned(),
            port: 8080,
            username: Some("user".to_owned()),
            password: Some("secret".to_owned()),
        }
    }

    /// Runs `git ls-remote` for the remote under `env` with `global_config` as the user's Git
    /// config, returning stderr so the caller can see which route Git took.
    fn ls_remote_stderr(env: GitEnv, global_config: &Path) -> String {
        let command = GitCommand::new(
            global_config.parent().expect("config parent").to_path_buf(),
            vec!["ls-remote".to_owned(), REMOTE_URL.to_owned()],
            env.with_variable("GIT_CONFIG_GLOBAL", global_config.to_string_lossy())
                .with_variable("GIT_CONFIG_NOSYSTEM", "1"),
            GitIntent::Network,
        );
        match CliGitRunner.run(&command) {
            Err(GitExecError::NonZeroExit { stderr, .. }) => stderr,
            other => panic!("expected ls-remote to fail, got {other:?}"),
        }
    }

    /// Verifies both routes pin the remote's origin and full URL, with credentials embedded only
    /// when proxying.
    #[test]
    fn git_proxy_env_pins_origin_and_full_url() {
        let settings = ora_proxy_settings();

        assert_eq!(
            (
                git_proxy_env(REMOTE_URL, GitProxyRoute::Direct).expect("direct env"),
                git_proxy_env(REMOTE_URL, GitProxyRoute::Proxy(&settings)).expect("proxy env"),
            ),
            (
                GitEnv::automation_defaults()
                    .with_config("http.https://example.invalid.proxy", "")
                    .with_config(format!("http.{REMOTE_URL}.proxy"), ""),
                GitEnv::automation_defaults()
                    .with_config(
                        "http.https://example.invalid.proxy",
                        "http://user:secret@ora-proxy.invalid:8080/",
                    )
                    .with_config(
                        format!("http.{REMOTE_URL}.proxy"),
                        "http://user:secret@ora-proxy.invalid:8080/",
                    ),
            )
        );
    }

    /// Verifies a direct route bypasses every proxy the user's Git config names.
    #[test]
    fn direct_route_overrides_user_git_proxy() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let config = temp.path().join("gitconfig");
        std::fs::write(&config, USER_PROXY_CONFIG).expect("write user config");
        let env = git_proxy_env(REMOTE_URL, GitProxyRoute::Direct).expect("direct env");

        let stderr = ls_remote_stderr(env, &config);

        assert!(
            stderr.contains("Could not resolve host: example.invalid"),
            "{stderr}"
        );
    }

    /// Verifies a proxied route uses Ora's proxy over every proxy the user's Git config names.
    #[test]
    fn proxy_route_overrides_user_git_proxy() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let config = temp.path().join("gitconfig");
        std::fs::write(&config, USER_PROXY_CONFIG).expect("write user config");
        let settings = ora_proxy_settings();
        let env = git_proxy_env(REMOTE_URL, GitProxyRoute::Proxy(&settings)).expect("proxy env");

        let stderr = ls_remote_stderr(env, &config);

        assert!(
            stderr.contains("Could not resolve proxy: ora-proxy.invalid"),
            "{stderr}"
        );
    }
}

use gitlancer::GitEnv;
use ora_application::{NetworkProxyMode, NetworkProxyPreferences, NetworkProxySettings};
use ora_contracts::CheckProxySettingsResponse;
use ora_utils::http::{HostProxy, HostProxyError, ReqwestDownloader};
use std::time::Duration;
use url::Url;

use crate::BackendError;

/// Time budget for one proxy connectivity probe, including connect and the first response headers.
const PROXY_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Creates the policy shared by Desktop updates, marketplace traffic and the settings probe.
pub(crate) fn host_proxy(preferences: &NetworkProxyPreferences) -> Result<HostProxy, BackendError> {
    match preferences.mode {
        NetworkProxyMode::Direct => Ok(HostProxy::direct()),
        NetworkProxyMode::System => HostProxy::system().map_err(proxy_error),
        NetworkProxyMode::Manual => {
            let settings = preferences
                .manual
                .as_ref()
                .ok_or_else(|| proxy_error(HostProxyError::InvalidEndpoint))?;
            Ok(HostProxy::manual(proxy_endpoint_with_credentials(
                settings,
            )?))
        }
    }
}

/// Produces a public proxy error without including any endpoint credentials.
pub(crate) fn proxy_error(error: HostProxyError) -> BackendError {
    BackendError::invalid_proxy_settings(error.to_string())
}

/// Pins a single Git operation to its selected route and disallows redirects Git cannot reroute.
pub(crate) fn git_proxy_env(remote_url: &str, policy: &HostProxy) -> Result<GitEnv, BackendError> {
    let url = Url::parse(remote_url)
        .map_err(|error| BackendError::internal("marketplace source URL is invalid", error))?;
    let origin = url.origin().ascii_serialization();
    let proxy_url = policy
        .route(&url)
        .map_err(proxy_error)?
        .endpoint
        .map(String::from)
        .unwrap_or_default();
    let env = GitEnv::automation_defaults()
        .with_english_messages()
        .with_variable("NO_PROXY", "")
        .with_variable("no_proxy", "")
        .with_config("http.proxyAuthMethod", "basic")
        .with_config(format!("http.{origin}.proxy"), proxy_url.as_str())
        .with_config(format!("http.{origin}.proxyAuthMethod"), "basic")
        .with_config(format!("http.{remote_url}.proxyAuthMethod"), "basic")
        .with_config(format!("http.{remote_url}.proxy"), proxy_url);
    if policy.is_always_direct() {
        // Git matches URL options against the original request. Empty proxy values pin its
        // transport to direct, so following an allowed redirect cannot change the route.
        return Ok(env
            .with_config("http.proxy", "")
            .with_config("remote.origin.proxy", ""));
    }
    Ok(env
        .with_config("http.followRedirects", "false")
        .with_config(format!("http.{origin}.followRedirects"), "false")
        .with_config(format!("http.{remote_url}.followRedirects"), "false"))
}

/// Returns the plain endpoint URL from user-provided proxy settings.
fn proxy_endpoint(settings: &NetworkProxySettings) -> Result<Url, BackendError> {
    let mut endpoint = Url::parse(&normalized_proxy_url(settings)?)
        .map_err(|_| proxy_error(HostProxyError::InvalidEndpoint))?;
    if !matches!(endpoint.scheme(), "http" | "https" | "socks5" | "socks5h")
        || endpoint.host().is_none()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(proxy_error(HostProxyError::InvalidEndpoint));
    }
    endpoint
        .set_port(Some(settings.port))
        .map_err(|_| proxy_error(HostProxyError::InvalidEndpoint))?;
    Ok(endpoint)
}

/// Returns a proxy URL with configured credentials embedded for Git's environment contract.
fn proxy_endpoint_with_credentials(settings: &NetworkProxySettings) -> Result<Url, BackendError> {
    let mut endpoint = proxy_endpoint(settings)?;
    if let Some(username) = &settings.username {
        endpoint
            .set_username(username)
            .map_err(|()| proxy_error(HostProxyError::InvalidEndpoint))?;
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

    Ok(if host.contains("://") {
        host.to_owned()
    } else {
        format!("http://{host}")
    })
}

/// Probes a target along its resolved route without persisting form edits.
pub(crate) async fn check_proxy(
    preferences: &NetworkProxyPreferences,
    url: &str,
) -> Result<CheckProxySettingsResponse, BackendError> {
    use ora_contracts::ProxyRoute;
    let unreachable = |message: String| CheckProxySettingsResponse::Unreachable { message };
    let Ok(parsed) = Url::parse(url) else {
        return Ok(unreachable("invalid probe URL".to_owned()));
    };
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Ok(unreachable(
            "URL must use HTTP(S) without credentials".to_owned(),
        ));
    }
    let policy = match host_proxy(preferences) {
        Ok(policy) => policy,
        Err(error) => return Ok(unreachable(error.to_string())),
    };
    let origin = match policy.route(&parsed) {
        Ok(route) => route.origin,
        Err(error) => return Ok(unreachable(error.to_string())),
    };
    let route = match origin {
        ora_utils::http::ProxyOrigin::Direct => ProxyRoute::Direct,
        ora_utils::http::ProxyOrigin::Manual => ProxyRoute::Manual,
        ora_utils::http::ProxyOrigin::System => ProxyRoute::System,
        ora_utils::http::ProxyOrigin::Environment => ProxyRoute::Environment,
    };
    match ReqwestDownloader::for_host_proxy(policy)
        .probe(parsed, PROXY_CHECK_TIMEOUT)
        .await
    {
        Ok(status) => Ok(CheckProxySettingsResponse::Reachable {
            status,
            route: Some(route),
        }),
        Err(error) => Ok(unreachable(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitlancer::{CliGitRunner, GitCommand, GitExecError, GitIntent, GitRunner};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;

    /// Runs a real Git network operation against a fully controlled local endpoint.
    fn ls_remote_stderr(remote: &str, env: GitEnv, global_config: &Path) -> String {
        let command = GitCommand::new(
            global_config.parent().expect("config parent").to_path_buf(),
            vec!["ls-remote".to_owned(), remote.to_owned()],
            env.with_variable("GIT_CONFIG_GLOBAL", global_config.to_string_lossy())
                .with_variable("GIT_CONFIG_NOSYSTEM", "1"),
            GitIntent::Network,
        )
        .with_time_limit(Duration::from_secs(10));
        match CliGitRunner.run(&command) {
            Err(GitExecError::NonZeroExit { stderr, .. }) => stderr,
            other => panic!("expected git ls-remote to fail, got {other:?}"),
        }
    }

    const UNAVAILABLE: &str =
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    /// Receives one Git request on a socket and returns its method and target.
    fn serve_git_request(response: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        listener.set_nonblocking(true).expect("set nonblocking");
        let endpoint = format!("http://{}", listener.local_addr().expect("fixture address"));
        let task = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed() < Duration::from_secs(5) =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("Git never reached the fixture: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            let mut reader = BufReader::new(&mut socket);
            let mut request = String::new();
            reader.read_line(&mut request).expect("read Git request");
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).expect("read header") == 0 || header == "\r\n" {
                    break;
                }
            }
            socket.write_all(response.as_bytes()).expect("reply to Git");
            request
        });
        (endpoint, task)
    }

    /// Direct Git traffic ignores URL-scoped Git settings and an inherited environment proxy.
    #[test]
    fn direct_route_overrides_user_git_proxy() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        let (endpoint, server) = serve_git_request(UNAVAILABLE.to_owned());
        let remote = format!("{endpoint}/marketplace.git");
        std::fs::write(
            &config,
            format!("[http \"{remote}\"]\n  proxy = http://127.0.0.1:1\n"),
        )
        .expect("git config");
        let env = git_proxy_env(&remote, &HostProxy::direct())
            .expect("direct env")
            .with_variable("http_proxy", "http://127.0.0.1:1");
        let stderr = ls_remote_stderr(&remote, env, &config);
        assert!(stderr.contains("503"), "{stderr}");
        let line = server.join().expect("fixture");
        assert!(
            line.starts_with("GET /marketplace.git/info/refs?"),
            "{line}"
        );
    }

    /// Git follows a repository redirect without consulting a conflicting proxy on either hop.
    #[test]
    fn direct_marketplace_git_follows_initial_redirect() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        let (destination, final_request) = serve_git_request(UNAVAILABLE.to_owned());
        let redirect = format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {destination}/moved.git/info/refs?service=git-upload-pack\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (initial, first_request) = serve_git_request(redirect);
        let remote = format!("{initial}/initial.git");
        std::fs::write(&config, format!(
            "[http]\n  proxy = http://127.0.0.1:1\n[http \"{destination}\"]\n  proxy = http://127.0.0.1:1\n"
        )).expect("git config");
        let env = git_proxy_env(&remote, &HostProxy::direct())
            .expect("direct env")
            .with_variable("http_proxy", "http://127.0.0.1:1");
        let stderr = ls_remote_stderr(&remote, env, &config);
        assert!(stderr.contains("503"), "{stderr}");
        assert!(
            first_request
                .join()
                .expect("initial request")
                .starts_with("GET /initial.git/info/refs?")
        );
        assert!(
            final_request
                .join()
                .expect("redirected request")
                .starts_with("GET /moved.git/info/refs?")
        );
    }

    /// A routed Git request reaches Ora's local proxy despite conflicting Git and bypass settings.
    #[test]
    fn proxy_route_overrides_user_git_proxy() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        let remote = "http://origin.example/marketplace.git";
        let (endpoint, server) = serve_git_request(UNAVAILABLE.to_owned());
        std::fs::write(
            &config,
            format!("[http \"{remote}\"]\n  proxy = http://127.0.0.1:1\n"),
        )
        .expect("git config");
        let policy = HostProxy::manual(Url::parse(&endpoint).expect("fixture URL"));
        let env = git_proxy_env(remote, &policy).expect("proxy env");
        let stderr = ls_remote_stderr(remote, env, &config);
        assert!(stderr.contains("503"), "{stderr}");
        let line = server.join().expect("fixture");
        assert!(
            line.starts_with("GET http://origin.example/marketplace.git/info/refs?"),
            "{line}"
        );
    }
}

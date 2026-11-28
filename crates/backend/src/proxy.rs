use gitlancer::GitEnv;
use ora_application::{NetworkProxyMode, NetworkProxyPreferences, NetworkProxySettings};
use ora_utils::http::{HostProxy, HostProxyError};
use url::Url;

use crate::BackendError;

/// Creates the policy shared by Desktop updates, marketplace traffic and the settings probe.
pub(crate) fn host_proxy(preferences: &NetworkProxyPreferences) -> Result<HostProxy, BackendError> {
    match preferences.mode {
        NetworkProxyMode::Direct => Ok(HostProxy::direct()),
        NetworkProxyMode::System => Ok(HostProxy::system()),
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

/// Recognizes Git's English HTTP and CONNECT proxy authentication diagnostics.
pub(crate) fn git_proxy_auth_failure(message: &str) -> bool {
    message.contains("CONNECT tunnel failed, response 407")
        || message.contains("returned error: 407")
}

/// Recognizes a redirect that Git refused with redirects disabled.
pub(crate) fn git_proxy_redirect_failure(message: &str) -> bool {
    message
        .match_indices("The requested URL returned error: ")
        .any(|(index, prefix)| {
            message[index + prefix.len()..]
                .split_whitespace()
                .next()
                .and_then(|status| status.parse::<u16>().ok())
                .is_some_and(|status| matches!(status, 301 | 302 | 303 | 307 | 308))
        })
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
        .with_config("remote.origin.proxy", proxy_url.as_str())
        .with_config(format!("http.{remote_url}.proxy"), proxy_url);
    if policy.is_always_direct() {
        // Git matches URL options against the original request. Empty proxy values pin its
        // transport to direct, so following an allowed redirect cannot change the route.
        return Ok(env.with_config("http.proxy", ""));
    }
    Ok(env
        .with_config("http.followRedirects", "false")
        .with_config(format!("http.{origin}.followRedirects"), "false")
        .with_config(format!("http.{remote_url}.followRedirects"), "false"))
}

/// Returns the plain endpoint URL from user-provided proxy settings.
fn proxy_endpoint(settings: &NetworkProxySettings) -> Result<Url, BackendError> {
    let mut endpoint = Url::parse(&normalized_proxy_url(settings)?)
        .map_err(|_| BackendError::invalid_proxy_settings("invalid proxy host".to_owned()))?;
    if !matches!(endpoint.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err(BackendError::invalid_proxy_settings(
            "proxy scheme must be HTTP, HTTPS, SOCKS5, or SOCKS5H".to_owned(),
        ));
    }
    if endpoint.host().is_none() {
        return Err(BackendError::invalid_proxy_settings(
            "proxy address must include a host".to_owned(),
        ));
    }
    if endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err(BackendError::invalid_proxy_settings(
            "proxy address must not include a path, query, or fragment".to_owned(),
        ));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(BackendError::invalid_proxy_settings(
            "enter proxy credentials in the username and password fields".to_owned(),
        ));
    }
    endpoint
        .set_port(Some(settings.port))
        .map_err(|_| proxy_error(HostProxyError::InvalidEndpoint))?;
    endpoint.set_path("");
    Ok(endpoint)
}

/// Validates the endpoint before saving and stores only the host and non-default scheme.
pub(crate) fn normalize_manual_settings(
    mut settings: NetworkProxySettings,
) -> Result<NetworkProxySettings, BackendError> {
    let endpoint = proxy_endpoint_with_credentials(&settings)?;
    let host = endpoint
        .host_str()
        .ok_or_else(|| proxy_error(HostProxyError::InvalidEndpoint))?;
    settings.host = match endpoint.scheme() {
        "http" => host.to_owned(),
        scheme => format!("{scheme}://{host}"),
    };
    Ok(settings)
}

/// Returns a proxy URL with configured credentials embedded for Git's environment contract.
fn proxy_endpoint_with_credentials(settings: &NetworkProxySettings) -> Result<Url, BackendError> {
    let mut endpoint = proxy_endpoint(settings)?;
    // URL setters preserve percent escapes, but the clients decode URL credentials.
    // Encode literal percent signs so those clients receive the saved field values.
    if let Some(username) = &settings.username {
        endpoint
            .set_username(&username.replace('%', "%25"))
            .map_err(|()| proxy_error(HostProxyError::InvalidEndpoint))?;
    }
    if let Some(password) = &settings.password {
        endpoint
            .set_password(Some(&password.replace('%', "%25")))
            .map_err(|()| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gitlancer::{CliGitRunner, GitCommand, GitExecError, GitIntent, GitRunner};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::time::Duration;

    /// Runs a real Git network operation against a fully controlled local endpoint.
    fn ls_remote_stderr(remote: &str, env: GitEnv, global_config: &Path) -> String {
        git_network_stderr(["ls-remote", remote], env, global_config)
    }

    /// Runs a fixture Git command and returns the diagnostic from its failed HTTP operation.
    fn git_network_stderr(args: [&str; 2], env: GitEnv, global_config: &Path) -> String {
        let command = GitCommand::new(
            global_config.parent().expect("config parent").to_path_buf(),
            args.into_iter().map(str::to_owned).collect(),
            env.with_variable("GIT_CONFIG_GLOBAL", global_config.to_string_lossy())
                .with_variable("GIT_CONFIG_NOSYSTEM", "1"),
            GitIntent::Network,
        )
        .with_time_limit(Duration::from_secs(10));
        match CliGitRunner.run(&command) {
            Err(GitExecError::NonZeroExit { stderr, .. }) => stderr,
            other => panic!("expected Git to fail, got {other:?}"),
        }
    }

    const UNAVAILABLE: &str =
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    /// Receives one HTTP request on a socket and returns its method and target.
    fn serve_http_request(response: String) -> (String, std::thread::JoinHandle<String>) {
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
                    Err(error) => panic!("no request reached the fixture: {error}"),
                }
            };
            // Windows can inherit the listener's nonblocking mode on accepted sockets.
            socket
                .set_nonblocking(false)
                .expect("blocking fixture socket");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            let mut reader = BufReader::new(&mut socket);
            let mut request = String::new();
            reader.read_line(&mut request).expect("read request");
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).expect("read header") == 0 || header == "\r\n" {
                    break;
                }
            }
            socket
                .write_all(response.as_bytes())
                .expect("reply to client");
            request
        });
        (endpoint, task)
    }

    /// Direct Git traffic ignores URL-scoped Git settings and an inherited environment proxy.
    #[test]
    fn direct_route_overrides_user_git_proxy() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        let (endpoint, server) = serve_http_request(UNAVAILABLE.to_owned());
        let remote = format!("{endpoint}/marketplace.git");
        assert!(
            std::process::Command::new("git")
                .arg("init")
                .arg("--quiet")
                .current_dir(temp.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(
            &config,
            format!("[http \"{remote}\"]\n  proxy = http://127.0.0.1:1\n[remote \"origin\"]\n  url = {remote}\n  proxy = http://127.0.0.1:1\n"),
        )
        .expect("git config");
        let env = git_proxy_env(&remote, &HostProxy::direct())
            .expect("direct env")
            .with_variable("http_proxy", "http://127.0.0.1:1");
        let stderr = git_network_stderr(["fetch", "origin"], env, &config);
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
        assert_marketplace_git_follows_redirect(&HostProxy::direct());
    }

    /// An operation with no effective system proxy can safely follow a Git redirect.
    #[cfg(not(windows))]
    #[test]
    fn system_without_a_proxy_follows_initial_git_redirect() {
        const CHILD: &str = "ORA_TEST_GIT_SYSTEM_NO_PROXY_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert_marketplace_git_follows_redirect(&HostProxy::system());
            return;
        }
        let name = format!(
            "{}::system_without_a_proxy_follows_initial_git_redirect",
            module_path!()
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name.split_once("::").unwrap().1])
            .env(CHILD, "1")
            .env("XDG_CURRENT_DESKTOP", "fixture")
            .env_remove("http_proxy")
            .env_remove("HTTP_PROXY")
            .env_remove("https_proxy")
            .env_remove("HTTPS_PROXY")
            .env_remove("all_proxy")
            .env_remove("ALL_PROXY")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}"
        );
    }

    /// Drives a real redirect with the selected policy and confirms both requests stay direct.
    fn assert_marketplace_git_follows_redirect(policy: &HostProxy) {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        let (destination, final_request) = serve_http_request(UNAVAILABLE.to_owned());
        let redirect = format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {destination}/moved.git/info/refs?service=git-upload-pack\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (initial, first_request) = serve_http_request(redirect);
        let remote = format!("{initial}/initial.git");
        std::fs::write(&config, format!(
            "[http]\n  proxy = http://127.0.0.1:1\n[http \"{destination}\"]\n  proxy = http://127.0.0.1:1\n"
        )).expect("git config");
        let env = git_proxy_env(&remote, policy)
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
        let (endpoint, server) = serve_http_request(UNAVAILABLE.to_owned());
        std::fs::write(
            &config,
            format!("[http \"{remote}\"]\n  proxy = http://127.0.0.1:1\n[remote \"origin\"]\n  url = {remote}\n  proxy = http://127.0.0.1:1\n"),
        )
        .expect("git config");
        let policy = HostProxy::manual(Url::parse(&endpoint).expect("fixture URL"));
        let env = git_proxy_env(remote, &policy).expect("proxy env");
        let stderr = ls_remote_stderr("origin", env, &config);
        assert!(stderr.contains("503"), "{stderr}");
        let line = server.join().expect("fixture");
        assert!(
            line.starts_with("GET http://origin.example/marketplace.git/info/refs?"),
            "{line}"
        );
    }

    /// Git cannot re-resolve an opted-in proxied request after a redirect.
    #[test]
    fn proxied_git_refuses_redirects() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        let remote = "http://origin.example/marketplace.git";
        let (endpoint, server) = serve_http_request(
            "HTTP/1.1 302 Found\r\nLocation: http://other.example/marketplace.git/info/refs\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
        );
        let policy = HostProxy::manual(Url::parse(&endpoint).expect("fixture URL"));
        let env = git_proxy_env(remote, &policy).expect("proxy env");
        let stderr = ls_remote_stderr(remote, env, &config);
        assert!(git_proxy_redirect_failure(&stderr), "{stderr}");
        assert!(
            server
                .join()
                .unwrap()
                .starts_with("GET http://origin.example/")
        );
    }

    /// Real Git diagnostics from both proxy handshakes are recognized by marketplace sync.
    #[test]
    fn git_reports_proxy_authentication_failure() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = temp.path().join("gitconfig");
        for (remote, method) in [
            ("http://origin.example/marketplace.git", "GET "),
            ("https://origin.example/marketplace.git", "CONNECT "),
        ] {
            let (endpoint, server) = serve_http_request(
                "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=ora\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
            );
            let policy = HostProxy::manual(Url::parse(&endpoint).expect("fixture URL"));
            let env = git_proxy_env(remote, &policy).expect("proxy env");
            let stderr = ls_remote_stderr(remote, env, &config);
            assert!(git_proxy_auth_failure(&stderr), "{stderr}");
            assert!(server.join().unwrap().starts_with(method));
        }
    }
}

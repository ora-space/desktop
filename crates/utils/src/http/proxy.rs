//! HTTP(S) proxy resolution for the download capability.
//!
//! Choosing a proxy follows explicit configuration, then per-scheme environment variables, then a
//! direct connection. Every resolution path honors an optional bypass list so matching hosts always
//! connect directly. The logic is pure and env-injectable so it can be unit-tested without touching
//! the process environment.

use std::net::IpAddr;
use url::Url;

/// A proxy endpoint, optionally carrying credentials for the tunnel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    /// The proxy URL; its scheme selects the protocol (`http://`, `https://`, `socks5://`).
    pub endpoint: Url,
    /// Optional tunnel credentials, kept in memory only and never logged.
    pub auth: Option<ProxyAuth>,
}

/// Credentials for a proxy, isolated from any persistent storage or logging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

/// Hosts and networks that bypass the proxy and connect directly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyBypass {
    patterns: Vec<String>,
}

impl ProxyBypass {
    /// Builds a bypass list from individual host, suffix, IP, or CIDR patterns.
    pub fn new(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns true when `url`'s host must connect directly.
    ///
    /// A pattern matches when it is `*`, an exact hostname, a leading-dot suffix, an IP address, or
    /// a CIDR network; loopback addresses always bypass the proxy.
    pub fn matches(&self, url: &Url) -> bool {
        host_is_bypassed(url, &self.patterns)
    }
}

/// Whether `url` should bypass the proxy according to `patterns`.
fn host_is_bypassed(url: &Url, patterns: &[String]) -> bool {
    if patterns.iter().any(|pattern| pattern == "*") {
        return true;
    }
    let Some(host) = url.host_str() else {
        return true;
    };
    if is_loopback(host) {
        return true;
    }
    let host_lower = host.to_ascii_lowercase();
    patterns
        .iter()
        .any(|pattern| pattern_matches(pattern, host, &host_lower))
}

/// Matches one `NO_PROXY`-style pattern against a hostname or IP.
fn pattern_matches(pattern: &str, host: &str, host_lower: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    if let Some(suffix) = pattern.strip_prefix('.') {
        return host_lower == suffix || host_lower.ends_with(&pattern);
    }
    if pattern.contains('/') {
        return cidr_matches(&pattern, host);
    }
    match (host.parse::<IpAddr>(), pattern.parse::<IpAddr>()) {
        (Ok(host_ip), Ok(pattern_ip)) => host_ip == pattern_ip,
        (Ok(_), Err(_)) => false,
        _ => host_lower == pattern || host_lower.ends_with(&format!(".{pattern}")),
    }
}

/// Checks whether `host` falls inside a `base/prefix` CIDR network.
fn cidr_matches(pattern: &str, host: &str) -> bool {
    let Some((base, prefix)) = pattern.split_once('/') else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    let Ok(host_ip) = host.parse::<IpAddr>() else {
        return false;
    };
    let Ok(network_ip) = base.parse::<IpAddr>() else {
        return false;
    };
    ip_prefix_matches(network_ip, host_ip, prefix)
}

/// Compares the first `prefix` bits of two IP addresses of the same family.
fn ip_prefix_matches(network: IpAddr, host: IpAddr, prefix: u8) -> bool {
    let (network_bits, host_bits) = match (network, host) {
        (IpAddr::V4(a), IpAddr::V4(b)) if prefix <= 32 => (u32::from(a), u32::from(b)),
        (IpAddr::V6(a), IpAddr::V6(b)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            return (u128::from(a) & mask) == (u128::from(b) & mask);
        }
        _ => return false,
    };
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    (network_bits & mask) == (host_bits & mask)
}

/// True for loopback hostnames and loopback IPs, which always connect directly.
fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    matches!(host.parse::<IpAddr>(), Ok(ip) if ip.is_loopback())
}

/// Controls how the proxy for a download is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyConfig {
    /// An explicit proxy used before any environment- or system-derived value.
    pub explicit: Option<Proxy>,
    /// Whether to honor `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`. Default true.
    pub use_env: bool,
    /// Whether to fall back to the platform system proxy. Default true.
    pub use_system: bool,
    /// Hosts that always connect directly.
    pub bypass: ProxyBypass,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            explicit: None,
            use_env: true,
            use_system: true,
            bypass: ProxyBypass::default(),
        }
    }
}

/// Resolves the proxy for `url`, or returns `None` to connect directly.
pub fn resolve_proxy(url: &Url, config: &ProxyConfig) -> Option<Proxy> {
    resolve_proxy_with(url, config, &|key| std::env::var(key).ok())
}

/// Resolves the proxy using an injected environment lookup, so tests never mutate process env.
fn resolve_proxy_with(
    url: &Url,
    config: &ProxyConfig,
    env: &impl Fn(&str) -> Option<String>,
) -> Option<Proxy> {
    let mut patterns = config.bypass.patterns.clone();
    if config.use_env
        && let Some(no_proxy) = env("no_proxy")
            .or_else(|| env("NO_PROXY"))
            .map(split_no_proxy)
    {
        patterns.extend(no_proxy);
    }
    if host_is_bypassed(url, &patterns) {
        return None;
    }
    if let Some(proxy) = &config.explicit {
        return Some(proxy.clone());
    }
    if config.use_env {
        return environment_proxy_for(url, env);
    }
    if config.use_system {
        return system_proxy_for(url);
    }
    None
}

/// Splits a `NO_PROXY` value into non-empty trimmed patterns.
fn split_no_proxy(value: String) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Reads the per-scheme proxy environment variable for `url`.
fn environment_proxy_for(url: &Url, env: &impl Fn(&str) -> Option<String>) -> Option<Proxy> {
    let keys: &[&str] = match url.scheme() {
        "https" => &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"],
        _ => &["http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"],
    };
    keys.iter()
        .find_map(|key| env(key).and_then(|value| parse_proxy(&value)))
}

/// Looks up the platform system proxy; currently only the environment path is implemented, so a
/// direct connection is returned until a platform-specific reader lands.
fn system_proxy_for(_url: &Url) -> Option<Proxy> {
    None
}

/// Parses a proxy environment value into a proxy endpoint, tolerating a missing scheme.
fn parse_proxy(value: &str) -> Option<Proxy> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let endpoint = Url::parse(value)
        .or_else(|_| Url::parse(&format!("http://{value}")))
        .ok()?;
    if !matches!(endpoint.scheme(), "http" | "https" | "socks5") {
        return None;
    }
    let auth = if endpoint.username().is_empty() && endpoint.password().is_none() {
        None
    } else {
        Some(ProxyAuth {
            username: endpoint.username().to_owned(),
            password: endpoint.password().unwrap_or_default().to_owned(),
        })
    };
    Some(Proxy { endpoint, auth })
}
#[cfg(test)]
mod tests {
    use super::{Proxy, ProxyAuth, ProxyBypass, ProxyConfig, resolve_proxy_with};
    use pretty_assertions::assert_eq;
    use url::Url;

    /// Resolves a proxy against an in-memory environment so tests never touch process env.
    fn resolve(url: &str, config: &ProxyConfig, env: &[(&str, &str)]) -> Option<Proxy> {
        resolve_proxy_with(&Url::parse(url).unwrap(), config, &|key| {
            env.iter()
                .find(|(candidate, _)| *candidate == key)
                .map(|(_, value)| value.to_string())
        })
    }

    fn no_proxy_env(no_proxy: &str) -> Vec<(&str, &str)> {
        vec![("no_proxy", no_proxy)]
    }

    /// An explicit proxy is used for any scheme unless the host is bypassed.
    #[test]
    fn explicit_proxy_used_for_both_schemes() {
        let config = ProxyConfig {
            explicit: Some(Proxy {
                endpoint: Url::parse("http://proxy:8080").unwrap(),
                auth: None,
            }),
            ..Default::default()
        };
        assert!(resolve("https://example.com/a", &config, &[]).is_some());
        assert!(resolve("http://example.com/b", &config, &[]).is_some());
    }

    /// An explicit proxy is skipped when the target host is in the bypass list.
    #[test]
    fn bypass_skips_explicit_proxy() {
        let config = ProxyConfig {
            explicit: Some(Proxy {
                endpoint: Url::parse("http://proxy:8080").unwrap(),
                auth: None,
            }),
            bypass: ProxyBypass::new(["example.com"]),
            ..Default::default()
        };
        assert!(resolve("https://example.com/a", &config, &[]).is_none());
        assert!(resolve("https://other.com/a", &config, &[]).is_some());
    }

    /// The per-scheme environment variable select the right proxy.
    #[test]
    fn environment_proxy_follows_scheme() {
        let config = ProxyConfig::default();
        let env = vec![
            ("http_proxy", "http://http-proxy:8080"),
            ("https_proxy", "http://https-proxy:8081"),
        ];
        assert_eq!(
            resolve("http://example.com/a", &config, &env)
                .unwrap()
                .endpoint,
            Url::parse("http://http-proxy:8080").unwrap()
        );
        assert_eq!(
            resolve("https://example.com/a", &config, &env)
                .unwrap()
                .endpoint,
            Url::parse("http://https-proxy:8081").unwrap()
        );
    }

    /// `NO_PROXY` overrides the environment proxy for matching hosts.
    #[test]
    fn no_proxy_bypasses_environment_proxy() {
        let config = ProxyConfig::default();
        let mut env = Vec::from([("http_proxy", "http://proxy:8080")]);
        env.extend(no_proxy_env("internal.example.com"));
        assert!(resolve("http://internal.example.com/a", &config, &env).is_none());
        assert!(resolve("http://public.example.com/a", &config, &env).is_some());
    }

    /// A wildcard `NO_PROXY` bypasses every host.
    #[test]
    fn wildcard_no_proxy_bypasses_all() {
        let config = ProxyConfig::default();
        let mut env = Vec::from([("http_proxy", "http://proxy:8080")]);
        env.extend(no_proxy_env("*"));
        assert!(resolve("http://example.com/a", &config, &env).is_none());
    }

    /// Loopback hosts always connect directly regardless of configuration.
    #[test]
    fn loopback_always_direct() {
        let config = ProxyConfig {
            explicit: Some(Proxy {
                endpoint: Url::parse("http://proxy:8080").unwrap(),
                auth: None,
            }),
            ..Default::default()
        };
        assert!(resolve("http://localhost/a", &config, &[]).is_none());
        assert!(resolve("http://127.0.0.1/a", &config, &[]).is_none());
    }

    /// A leading-dot suffix pattern matches the domain and its subdomains.
    #[test]
    fn suffix_pattern_matches_subdomains() {
        let config = ProxyConfig::default();
        let mut env = Vec::from([("http_proxy", "http://proxy:8080")]);
        env.extend(no_proxy_env(".example.com"));
        assert!(resolve("http://a.example.com/x", &config, &env).is_none());
        assert!(resolve("http://example.com/x", &config, &env).is_none());
        assert!(resolve("http://notexample.com/x", &config, &env).is_some());
    }

    /// A CIDR pattern bypasses any IP inside the network.
    #[test]
    fn cidr_pattern_matches_ip() {
        let config = ProxyConfig::default();
        let mut env = Vec::from([("http_proxy", "http://proxy:8080")]);
        env.extend(no_proxy_env("10.0.0.0/8"));
        assert!(resolve("http://10.1.2.3/x", &config, &env).is_none());
        assert!(resolve("http://192.168.1.1/x", &config, &env).is_some());
    }

    /// Disabling environment lookup leaves only the explicit proxy and direct connections.
    #[test]
    fn use_env_false_ignores_environment() {
        let config = ProxyConfig {
            use_env: false,
            ..Default::default()
        };
        let env = vec![("http_proxy", "http://proxy:8080")];
        assert!(resolve("http://example.com/a", &config, &env).is_none());
    }

    /// Proxy credentials are captured from the endpoint URL.
    #[test]
    fn parses_proxy_credentials() {
        let config = ProxyConfig::default();
        let env = vec![("http_proxy", "http://user:pass@proxy:8080")];
        let proxy = resolve("http://example.com/a", &config, &env).unwrap();
        assert_eq!(
            proxy.auth,
            Some(ProxyAuth {
                username: "user".to_owned(),
                password: "pass".to_owned(),
            })
        );
    }
}

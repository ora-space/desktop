#[cfg(not(windows))]
use std::env;
use thiserror::Error;
use url::Url;

use super::ProxyBypass;

#[cfg(all(target_os = "linux", feature = "desktop-proxy"))]
mod linux;
#[cfg(windows)]
mod windows;

/// The route actually selected for the destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyOrigin {
    Direct,
    Manual,
    System,
    Environment,
}

/// A per-target route, with credentials kept out of diagnostics.
pub struct HostRoute {
    pub origin: ProxyOrigin,
    pub endpoint: Option<Url>,
}

#[derive(Clone, Copy, Debug, Error)]
pub enum HostProxyError {
    #[error("could not read the system proxy settings")]
    SystemRead,
    #[error("automatic system proxy configuration (PAC/WPAD) is unsupported")]
    Automatic,
    #[error("invalid or unsupported proxy endpoint")]
    InvalidEndpoint,
    #[error("proxy authentication failed (HTTP 407)")]
    Authentication,
}

/// One operation's proxy configuration. Redirects select a route from the same snapshot.
#[derive(Clone)]
pub struct HostProxy {
    settings: Result<StaticProxy, HostProxyError>,
    origin: ProxyOrigin,
}

impl std::fmt::Debug for HostProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostProxy")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// Recognizes HTTPS CONNECT authentication failures reported by the HTTP connector.
pub fn is_proxy_auth_failure(error: &dyn std::error::Error) -> bool {
    let mut cause = Some(error);
    while let Some(current) = cause {
        if matches!(
            current.to_string().as_str(),
            "proxy authorization required" | "tunnel error: proxy authorization required"
        ) {
            return true;
        }
        cause = current.source();
    }
    false
}

#[derive(Clone, Default)]
struct StaticProxy {
    http: Option<String>,
    https: Option<String>,
    default: Option<String>,
    bypass: Vec<String>,
}

impl HostProxy {
    /// Creates a policy that ignores every ambient proxy source.
    pub fn direct() -> Self {
        Self {
            settings: Ok(StaticProxy::default()),
            origin: ProxyOrigin::Direct,
        }
    }

    /// Reports whether the policy guarantees a direct route regardless of the destination.
    pub fn is_always_direct(&self) -> bool {
        self.origin == ProxyOrigin::Direct
    }

    /// Uses the explicitly configured endpoint for non-loopback hosts.
    pub fn manual(endpoint: Url) -> Self {
        Self {
            settings: Ok(StaticProxy {
                default: Some(endpoint.into()),
                ..StaticProxy::default()
            }),
            origin: ProxyOrigin::Manual,
        }
    }

    /// Snapshots current-user settings or the Linux environment. A native read failure is
    /// kept in the snapshot so a request that never needs the proxy can still go direct.
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            Self {
                settings: windows::read(),
                origin: ProxyOrigin::System,
            }
        }
        #[cfg(not(windows))]
        {
            #[cfg(all(target_os = "linux", feature = "desktop-proxy"))]
            match linux::read() {
                Ok(Some(settings)) => {
                    return Self {
                        settings: Ok(settings),
                        origin: ProxyOrigin::System,
                    };
                }
                Err(error) => {
                    return Self {
                        settings: Err(error),
                        origin: ProxyOrigin::System,
                    };
                }
                Ok(None) => {}
            }
            Self {
                settings: Ok(StaticProxy {
                    http: env_proxy("http_proxy", "HTTP_PROXY"),
                    https: env_proxy("https_proxy", "HTTPS_PROXY"),
                    default: env_proxy("all_proxy", "ALL_PROXY"),
                    bypass: env_proxy("no_proxy", "NO_PROXY")
                        .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
                        .unwrap_or_default(),
                }),
                origin: ProxyOrigin::Environment,
            }
        }
    }

    /// Selects the proxy for one HTTP(S) hop, preserving explicit bypasses.
    pub fn route(&self, url: &Url) -> Result<HostRoute, HostProxyError> {
        let loopback = match url.host() {
            Some(url::Host::Domain(host)) => {
                host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
            }
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        if loopback {
            return Ok(HostRoute {
                origin: ProxyOrigin::Direct,
                endpoint: None,
            });
        }
        let settings = self.settings.as_ref().map_err(|error| *error)?;
        let bypassed = ProxyBypass::new(settings.bypass.iter().cloned()).matches(url)
            || url.host_str().is_some_and(|host| {
                let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
                settings.bypass.iter().any(|pattern| {
                    let pattern = pattern.trim().to_ascii_lowercase();
                    (pattern == "<local>" && !host.contains('.') && !host.contains(':'))
                        || (pattern.contains('*') && wildcard_matches(&pattern, &host))
                })
            });
        let endpoint = if bypassed {
            None
        } else {
            match url.scheme() {
                "http" => settings.http.as_deref(),
                "https" => settings.https.as_deref(),
                _ => return Err(HostProxyError::InvalidEndpoint),
            }
            .or(settings.default.as_deref())
            .map(proxy_endpoint)
            .transpose()?
        };
        Ok(HostRoute {
            origin: if endpoint.is_some() {
                self.origin
            } else {
                ProxyOrigin::Direct
            },
            endpoint,
        })
    }
}

/// Matches system bypass patterns with embedded asterisks.
fn wildcard_matches(pattern: &str, mut host: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(remainder) = host.strip_prefix(first) else {
        return false;
    };
    host = remainder;
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return host.ends_with(part);
        }
        let Some(index) = host.find(part) else {
            return false;
        };
        host = &host[index + part.len()..];
    }
    host.is_empty()
}

/// Parses an endpoint without putting the raw value or credentials into an error.
fn proxy_endpoint(raw: &str) -> Result<Url, HostProxyError> {
    let prefixed;
    let raw = if raw.contains("://") {
        raw
    } else {
        prefixed = format!("http://{raw}");
        &prefixed
    };
    let endpoint = Url::parse(raw).map_err(|_| HostProxyError::InvalidEndpoint)?;
    if !matches!(endpoint.scheme(), "http" | "https" | "socks5" | "socks5h")
        || endpoint.host().is_none()
        || endpoint.port_or_known_default().is_none()
        || !matches!(endpoint.path(), "" | "/")
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(HostProxyError::InvalidEndpoint);
    }
    Ok(endpoint)
}

/// Reads one conventional environment variable and its uppercase equivalent.
#[cfg(not(windows))]
fn env_proxy(lower: &str, upper: &str) -> Option<String> {
    env::var(lower)
        .ok()
        .or_else(|| env::var(upper).ok())
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// A broken native proxy setting cannot block loopback or become a public direct route.
    #[test]
    fn host_proxy_loopback_bypasses_resolution_errors() {
        for error in [HostProxyError::Automatic, HostProxyError::SystemRead] {
            let policy = HostProxy {
                settings: Err(error),
                origin: ProxyOrigin::System,
            };
            for address in [
                "http://localhost/",
                "http://localhost./",
                "http://127.0.0.1/",
                "http://[::1]/",
            ] {
                let route = policy
                    .route(&address.parse().expect("loopback URL"))
                    .expect("direct route");
                assert_eq!((route.origin, route.endpoint), (ProxyOrigin::Direct, None));
            }
            assert!(matches!(
                policy.route(&"https://github.com/".parse().expect("public URL")),
                Err(HostProxyError::Automatic | HostProxyError::SystemRead)
            ));
        }
    }
}

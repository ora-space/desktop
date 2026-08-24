use ora_plugin_manifest::Origin;
use url::Url;

/// Data-driven navigation boundary of one surface webview.
///
/// A remote site may move within an exact set of HTTPS origins; a workbench page may only load
/// the host-served assets of its own instance. Both variants are pure data so the two Tauri
/// mount paths provably enforce the same rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationPolicy {
    /// A URL is allowed only when it is `https`, carries no credentials, and its normalized
    /// origin equals one of the declared origins. There is no wildcard or suffix form: which
    /// origins a login flow visits is a fact the plugin author can enumerate.
    RemoteSite { allowed_origins: Vec<Origin> },
    /// Only URLs below `base` (same scheme, host, and port; path prefix) are allowed; `base` is
    /// the per-instance asset URL the host serves the page from.
    WorkbenchAssets { base: Url },
}

impl NavigationPolicy {
    /// Builds a remote-site policy from validated manifest origins.
    pub fn remote_site(allowed_origins: Vec<Origin>) -> Self {
        Self::RemoteSite { allowed_origins }
    }

    /// Builds a workbench policy pinned to one asset base URL.
    pub fn workbench_assets(base: Url) -> Self {
        Self::WorkbenchAssets { base }
    }

    /// Decides whether a main-frame navigation may proceed.
    ///
    /// Credentials are refused even for allowed origins because the origin set describes a
    /// public site boundary. An empty origin set is fail-closed, so a misconfigured policy can
    /// never become "allow everything".
    pub fn allows(&self, url: &Url) -> bool {
        match self {
            Self::RemoteSite { allowed_origins } => {
                // Sites start in-page downloads by navigating to `blob:<origin>/<id>` URLs created
                // from a fetched response. The blob's origin is the page that minted it, so the
                // same origin rules apply to the inner URL; `blob:` alone never grants anything.
                if url.scheme() == "blob" {
                    return Url::parse(url.path()).is_ok_and(|origin| self.allows(&origin));
                }
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                {
                    return false;
                }
                allowed_origins.iter().any(|origin| origin.matches(url))
            }
            Self::WorkbenchAssets { base } => {
                url.scheme() == base.scheme()
                    && url.host_str() == base.host_str()
                    && url.port() == base.port()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.path().starts_with(base.path())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NavigationPolicy;
    use ora_plugin_manifest::Origin;
    use pretty_assertions::assert_eq;
    use url::Url;

    /// Builds the example-site exact-origin policy.
    fn example_policy() -> NavigationPolicy {
        NavigationPolicy::remote_site(origins(&[
            "https://example.com",
            "https://www.example.com",
            "https://sso.example.com:8443",
        ]))
    }

    /// Parses validated origins for fixtures.
    fn origins(values: &[&str]) -> Vec<Origin> {
        values
            .iter()
            .map(|value| Origin::parse(value).expect("valid origin"))
            .collect()
    }

    /// Parses a test URL with a failure message that preserves the invalid fixture.
    fn parse_url(value: &str) -> Url {
        Url::parse(value).unwrap_or_else(|error| panic!("parse test URL {value}: {error}"))
    }

    /// Exact origins allow paths, queries, blobs minted by them, and an explicitly declared port,
    /// and refuse everything else: subdomains, lookalikes, http, credentials, undeclared ports.
    #[test]
    fn remote_site_matches_exact_origins_only() {
        let policy = example_policy();
        let allowed = [
            "https://example.com",
            "https://WWW.example.com/skills/example?tab=install#x",
            "blob:https://www.example.com/ea83c2ef-e61d-47fe-b0b5-10b5ccb2246d",
            "https://sso.example.com:8443/login",
        ]
        .map(|value| policy.allows(&parse_url(value)));
        let denied = [
            "https://api.example.com/",
            "https://example.com.evil.test/",
            "http://www.example.com/",
            "https://user:pw@www.example.com/",
            "https://www.example.com:8443/",
            "https://sso.example.com/",
            "blob:https://evil.example/uuid",
            "blob:not-a-url",
            "file:///etc/passwd",
        ]
        .map(|value| policy.allows(&parse_url(value)));
        assert_eq!((allowed, denied), ([true; 4], [false; 9]));
    }

    /// An empty origin set allows nothing.
    #[test]
    fn empty_remote_policy_is_fail_closed() {
        let policy = NavigationPolicy::remote_site(vec![]);
        assert_eq!(policy.allows(&parse_url("https://example.com/")), false);
    }

    /// Workbench pages may only load below their own asset base.
    #[test]
    fn workbench_assets_allow_only_the_instance_base() {
        let policy = NavigationPolicy::workbench_assets(parse_url("ora-plugin://localhost/7/"));
        let allowed = [
            "ora-plugin://localhost/7/",
            "ora-plugin://localhost/7/index.html",
            "ora-plugin://localhost/7/app/app.js?v=1",
        ]
        .map(|value| policy.allows(&parse_url(value)));
        let denied = [
            "ora-plugin://localhost/70/index.html",
            "ora-plugin://localhost/8/index.html",
            "ora-plugin://evil/7/index.html",
            "https://localhost/7/index.html",
            "ora-plugin://user@localhost/7/",
        ]
        .map(|value| policy.allows(&parse_url(value)));
        assert_eq!((allowed, denied), ([true; 3], [false; 5]));
    }
}

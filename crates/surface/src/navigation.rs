use ora_plugin_manager::HostName;
use url::Url;

/// Data-driven navigation boundary of one remote site surface.
///
/// Generalizes the former hard-coded marketplace policy: a URL is allowed only when it is
/// `https`, carries no credentials and no explicit port, and its host either equals an exact
/// entry or is a subdomain of a suffix entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationPolicy {
    exact_hosts: Vec<HostName>,
    host_suffixes: Vec<HostName>,
}

impl NavigationPolicy {
    /// Builds a policy from validated manifest allow lists.
    pub fn new(exact_hosts: Vec<HostName>, host_suffixes: Vec<HostName>) -> Self {
        Self {
            exact_hosts,
            host_suffixes,
        }
    }

    /// Decides whether a navigation or new-window request may proceed.
    ///
    /// Ports and credentials are refused even for allowed hosts because the allow list describes
    /// a public site boundary, and a lookalike such as `allowed.com.evil.example` fails because
    /// suffix matching respects label boundaries.
    pub fn allows(&self, url: &Url) -> bool {
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        self.exact_hosts.iter().any(|exact| exact.as_str() == host)
            || self
                .host_suffixes
                .iter()
                .any(|suffix| suffix.matches_suffix_of(host))
    }
}

#[cfg(test)]
mod tests {
    use super::NavigationPolicy;
    use ora_plugin_manager::HostName;
    use pretty_assertions::assert_eq;
    use url::Url;

    /// Builds the SkillHub-style exact-host policy used by the migrated marketplace tests.
    fn skillhub_policy() -> NavigationPolicy {
        NavigationPolicy::new(hosts(&["skillhub.cn", "www.skillhub.cn"]), vec![])
    }

    /// Builds the Huawei-style suffix policy used by the migrated marketplace tests.
    fn huawei_policy() -> NavigationPolicy {
        NavigationPolicy::new(vec![], hosts(&["huawei.com"]))
    }

    /// Parses validated host names for fixtures.
    fn hosts(values: &[&str]) -> Vec<HostName> {
        values
            .iter()
            .map(|value| HostName::parse(value).expect("valid host"))
            .collect()
    }

    /// Parses a test URL with a failure message that preserves the invalid fixture.
    fn parse_url(value: &str) -> Url {
        Url::parse(value).unwrap_or_else(|error| panic!("parse test URL {value}: {error}"))
    }

    /// Verifies canonical exact-host navigation, including paths and queries, is allowed.
    #[test]
    fn allows_canonical_skillhub_navigation() {
        let policy = skillhub_policy();
        assert_eq!(
            [
                "https://skillhub.cn",
                "https://www.skillhub.cn/skills/example?tab=install",
            ]
            .map(parse_url)
            .map(|url| policy.allows(&url)),
            [true, true],
        );
    }

    /// Verifies suffix policies accept the apex and every subdomain over plain HTTPS.
    #[test]
    fn allows_huawei_internal_navigation() {
        let policy = huawei_policy();
        assert_eq!(
            [
                "https://ai.edevops.huawei.com/mcp/projects",
                "https://sso.huawei.com/login",
                "https://huawei.com/callback",
            ]
            .map(parse_url)
            .map(|url| policy.allows(&url)),
            [true, true, true],
        );
    }

    /// Verifies lookalikes, credentials, custom ports, insecure and non-web schemes are rejected.
    #[test]
    fn rejects_untrusted_navigation() {
        let skillhub = skillhub_policy();
        let huawei = huawei_policy();
        assert_eq!(
            [
                "http://www.skillhub.cn",
                "https://www.skillhub.cn.evil.example",
                "https://user@www.skillhub.cn",
                "https://www.skillhub.cn:8443",
                "https://example.com",
                "https://WWW.SKILLHUB.CN.evil.example",
                "about:blank",
                "javascript:alert(1)",
                "data:text/html,hi",
                "https://www.skillhüb.cn",
            ]
            .map(parse_url)
            .map(|url| skillhub.allows(&url)),
            [false; 10],
        );
        assert_eq!(
            [
                "http://ai.edevops.huawei.com/mcp/projects",
                "https://huawei.com.evil.example/login",
                "https://user@ai.edevops.huawei.com/mcp/projects",
                "https://sso.huawei.com:8443/login",
                "https://example.com",
                "https://nothuawei.com",
            ]
            .map(parse_url)
            .map(|url| huawei.allows(&url)),
            [false; 6],
        );
    }

    /// Verifies the URL parser lowercases ASCII hosts, so uppercase spellings of an allowed host
    /// still match, while an explicit default port is normalized away and therefore allowed.
    #[test]
    fn normalizes_host_case_and_default_port() {
        let policy = skillhub_policy();
        assert_eq!(
            ["https://WWW.SkillHub.CN/", "https://www.skillhub.cn:443/"]
                .map(parse_url)
                .map(|url| policy.allows(&url)),
            [true, true],
        );
    }

    /// Verifies an empty policy allows nothing, so a misconfigured surface fails closed.
    #[test]
    fn empty_policy_allows_nothing() {
        let policy = NavigationPolicy::new(vec![], vec![]);
        assert_eq!(policy.allows(&parse_url("https://example.com")), false);
    }
}

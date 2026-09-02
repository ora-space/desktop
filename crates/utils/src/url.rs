//! Canonicalizes repository URLs so two spellings of the same remote compare equal.
//!
//! Normalization is a pure, byte-stable transformation with no domain vocabulary: callers that
//! key durable state on a remote (checkout directories, derived identifiers) need the same URL
//! written with a different case, a `.git` suffix, a default port, or embedded credentials to
//! collapse onto one value, on every platform and in every version.

/// Ports that carry no information because they are the scheme's default.
const DEFAULT_PORTS: &[(&str, &str)] = &[
    ("https", "443"),
    ("http", "80"),
    ("ssh", "22"),
    ("git", "9418"),
];

/// Returns the canonical spelling of one repository URL.
///
/// The transformation is deliberately lossy in exactly one direction — the whole string is
/// lowercased, path included — because mainstream Git hosts treat owner and repository names
/// case-insensitively. Splitting one repository into two identities over a capital letter is a
/// worse failure than merging two repositories that differ only in case, which no mainstream
/// host can even host side by side.
///
/// Applied in order: lowercase, drop `userinfo@`, drop the scheme's default port, drop trailing
/// slashes, drop one trailing `.git`, drop trailing slashes again. A URL without a recognizable
/// `scheme://` prefix is normalized as a bare authority and path, so an unusual remote spelling
/// still canonicalizes deterministically instead of being passed through untouched.
pub fn canonical_repository_url(url: &str) -> String {
    let lowercased = url.trim().to_ascii_lowercase();
    let (scheme, rest) = match lowercased.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_owned()), rest.to_owned()),
        None => (None, lowercased),
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (rest[..index].to_owned(), rest[index..].to_owned()),
        None => (rest, String::new()),
    };
    let authority = strip_default_port(strip_userinfo(&authority), scheme.as_deref());
    let path = strip_trailing_slashes(&path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = strip_trailing_slashes(path);

    match scheme {
        Some(scheme) => format!("{scheme}://{authority}{path}"),
        None => format!("{authority}{path}"),
    }
}

/// Returns the last path segment of a canonical URL, empty when the URL carries no path.
///
/// The last segment is the only human-readable fragment that means "the repository" on every
/// hosting platform: path depth varies by host, and the segments before it can be an
/// organization, a nested subgroup, or a platform literal such as `_git` or `scm`. The authority
/// is deliberately not a fallback — a URL that names no repository has no readable name to take.
pub fn repository_url_last_segment(canonical_url: &str) -> &str {
    let rest = canonical_url
        .split_once("://")
        .map_or(canonical_url, |(_scheme, rest)| rest);
    let Some(path) = rest.find('/').map(|index| &rest[index..]) else {
        return "";
    };
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or_default()
}

/// Drops everything up to and including the last `@` of an authority.
///
/// Credentials are caller state rather than remote identity, so the same repository accessed
/// with and without a token must canonicalize to one value.
fn strip_userinfo(authority: &str) -> &str {
    match authority.rsplit_once('@') {
        Some((_userinfo, host)) => host,
        None => authority,
    }
}

/// Drops a trailing `:port` when it equals `scheme`'s default port.
///
/// The port is searched after any `]` so an IPv6 literal's internal colons are never mistaken
/// for a port separator.
fn strip_default_port<'a>(authority: &'a str, scheme: Option<&str>) -> &'a str {
    let Some(scheme) = scheme else {
        return authority;
    };
    let Some((host, port)) = split_port(authority) else {
        return authority;
    };
    let is_default = DEFAULT_PORTS
        .iter()
        .any(|(known_scheme, default_port)| *known_scheme == scheme && *default_port == port);
    if is_default { host } else { authority }
}

/// Splits `host:port` while tolerating a bracketed IPv6 literal.
fn split_port(authority: &str) -> Option<(&str, &str)> {
    let search_from = authority.rfind(']').map_or(0, |index| index + 1);
    let colon = authority[search_from..].find(':')? + search_from;
    Some((&authority[..colon], &authority[colon + 1..]))
}

/// Removes every trailing `/` so `…/repo/`, `…/repo//`, and `…/repo` agree.
fn strip_trailing_slashes(path: &str) -> &str {
    path.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::{canonical_repository_url, repository_url_last_segment};
    use pretty_assertions::assert_eq;

    /// Verifies every equivalent spelling of one repository collapses onto the same canonical URL.
    #[test]
    fn equivalent_spellings_canonicalize_to_one_url() {
        let expected = "https://github.com/acme/awesome-plugins";
        let equivalents = [
            "https://github.com/acme/awesome-plugins",
            "https://GitHub.com/Acme/Awesome-Plugins.git/",
            "https://github.com:443/acme/awesome-plugins/",
            "https://token@github.com/acme/awesome-plugins.git",
            "https://user:pass@GitHub.com:443/Acme/Awesome-Plugins.git///",
            "  https://github.com/acme/awesome-plugins.git  ",
        ];

        assert_eq!(
            equivalents
                .iter()
                .map(|url| canonical_repository_url(url))
                .collect::<Vec<_>>(),
            vec![expected.to_string(); equivalents.len()],
        );
    }

    /// Verifies a non-default port, a bracketed IPv6 host, and a scheme-less remote survive.
    #[test]
    fn keeps_information_bearing_authority_parts() {
        assert_eq!(
            [
                canonical_repository_url("https://git.example.com:8443/scm/infra/tooling.git"),
                canonical_repository_url("https://[2001:DB8::1]:443/acme/repo/"),
                canonical_repository_url("git.example.com/acme/repo.git"),
                canonical_repository_url("ssh://git@github.com:22/acme/repo.git"),
            ],
            [
                "https://git.example.com:8443/scm/infra/tooling".to_string(),
                "https://[2001:db8::1]/acme/repo".to_string(),
                "git.example.com/acme/repo".to_string(),
                "ssh://github.com/acme/repo".to_string(),
            ],
        );
    }

    /// Verifies the last path segment is extracted across differing platform path depths.
    #[test]
    fn extracts_the_last_path_segment() {
        let cases = [
            ("https://github.com/ora-space/desktop", "desktop"),
            (
                "https://dev.azure.com/contoso/analytics/_git/data-pipeline",
                "data-pipeline",
            ),
            (
                "https://gitlab.com/acme/platform/tools/orax-registry",
                "orax-registry",
            ),
            ("https://git.example.com:8443", ""),
            ("git.example.com/acme/repo", "repo"),
        ];
        for (url, expected) in cases {
            assert_eq!(repository_url_last_segment(url), expected, "{url}");
        }
    }
}

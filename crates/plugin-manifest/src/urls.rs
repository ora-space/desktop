use std::{fmt, str::FromStr};
use thiserror::Error;
use url::Url;

const MAX_URL_BYTES: usize = 2048;

#[derive(Clone, Copy)]
enum QueryPolicy {
    Allow,
    Reject,
}

/// Holds a validated HTTPS release URL, including an optional signing query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseUrl(Url);

impl ReleaseUrl {
    /// Parses an HTTPS release URL while allowing a query for signed downloads.
    pub fn parse(value: &str) -> Result<Self, UrlError> {
        parse_https_url(value, QueryPolicy::Allow).map(Self)
    }

    /// Returns the parsed URL value.
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// Returns the normalized URL string produced by the URL parser.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ReleaseUrl {
    /// Writes the normalized release URL.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ReleaseUrl {
    type Err = UrlError;

    /// Parses a release URL through its field-specific policy.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Holds a validated HTTPS homepage URL with no query or fragment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HomepageUrl(Url);

impl HomepageUrl {
    /// Parses an HTTPS homepage URL and rejects query parameters.
    pub fn parse(value: &str) -> Result<Self, UrlError> {
        parse_https_url(value, QueryPolicy::Reject).map(Self)
    }

    /// Returns the parsed URL value.
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// Returns the normalized URL string produced by the URL parser.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for HomepageUrl {
    /// Writes the normalized homepage URL.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for HomepageUrl {
    type Err = UrlError;

    /// Parses a homepage URL through its field-specific policy.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Holds a validated HTTPS source repository URL with no query or fragment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryUrl(Url);

impl RepositoryUrl {
    /// Parses an HTTPS repository URL without imposing host or `.git` suffix restrictions.
    pub fn parse(value: &str) -> Result<Self, UrlError> {
        parse_https_url(value, QueryPolicy::Reject).map(Self)
    }

    /// Returns the parsed URL value.
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// Returns the normalized URL string produced by the URL parser.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for RepositoryUrl {
    /// Writes the normalized repository URL.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RepositoryUrl {
    type Err = UrlError;

    /// Parses a repository URL through its field-specific policy.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Describes why an HTTPS manifest URL was rejected.
#[derive(Debug, Error)]
pub enum UrlError {
    #[error("URL exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("URL syntax is invalid: {0}")]
    InvalidSyntax(#[source] url::ParseError),
    #[error("URL scheme must be HTTPS")]
    NotHttps,
    #[error("URL must not contain a username or password")]
    CredentialsNotAllowed,
    #[error("URL must not contain a fragment")]
    FragmentNotAllowed,
    #[error("URL must not contain a query")]
    QueryNotAllowed,
}

/// Unwraps a Markdown `[label](target)` value into its embedded `target`, leaving strings that
/// do not use that wrapper unchanged.
///
/// Marketplace authors write link-valued fields as Markdown links, so the validators strip the
/// wrapper before applying HTTPS invariants to the embedded URL.
fn strip_markdown_link(value: &str) -> &str {
    let Some(rest) = value.strip_prefix('[') else {
        return value;
    };
    let Some(close_bracket) = rest.find("](") else {
        return value;
    };
    let Some(target) = rest[close_bracket + 2..].strip_suffix(')') else {
        return value;
    };
    target
}

/// Applies common HTTPS URL invariants plus one field-specific query policy.
fn parse_https_url(value: &str, query_policy: QueryPolicy) -> Result<Url, UrlError> {
    let value = strip_markdown_link(value);
    if value.len() > MAX_URL_BYTES {
        return Err(UrlError::TooLong {
            max_bytes: MAX_URL_BYTES,
            actual_bytes: value.len(),
        });
    }

    let parsed = Url::parse(value).map_err(UrlError::InvalidSyntax)?;
    if parsed.scheme() != "https" {
        return Err(UrlError::NotHttps);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(UrlError::CredentialsNotAllowed);
    }
    if parsed.fragment().is_some() {
        return Err(UrlError::FragmentNotAllowed);
    }
    if matches!(query_policy, QueryPolicy::Reject) && parsed.query().is_some() {
        return Err(UrlError::QueryNotAllowed);
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{HomepageUrl, ReleaseUrl, RepositoryUrl, UrlError};

    /// Verifies release URLs alone permit signing queries.
    #[test]
    fn applies_field_specific_query_policies() {
        let value = "https://example.com/plugin.orax?signature=abc";

        assert!(ReleaseUrl::parse(value).is_ok());
        assert!(matches!(
            HomepageUrl::parse(value),
            Err(UrlError::QueryNotAllowed)
        ));
        assert!(matches!(
            RepositoryUrl::parse(value),
            Err(UrlError::QueryNotAllowed)
        ));
    }

    /// Verifies common scheme, credential, and fragment restrictions apply to every URL type.
    #[test]
    fn rejects_common_url_policy_violations() {
        assert!(matches!(
            ReleaseUrl::parse("http://example.com/plugin.orax"),
            Err(UrlError::NotHttps)
        ));
        assert!(matches!(
            ReleaseUrl::parse("https://user@example.com/plugin.orax"),
            Err(UrlError::CredentialsNotAllowed)
        ));
        assert!(matches!(
            ReleaseUrl::parse("https://example.com/plugin.orax#digest"),
            Err(UrlError::FragmentNotAllowed)
        ));
    }

    /// Verifies Markdown-wrapped URLs (as marketplace authors write them) unwrap before validation.
    #[test]
    fn unwraps_markdown_linked_urls() {
        let homepage = "[https://example.com/ora-weather](https://example.com/ora-weather)";
        assert_eq!(
            HomepageUrl::parse(homepage).expect("homepage").as_str(),
            "https://example.com/ora-weather"
        );

        let release = "[https://example.com/plugin.orax](https://example.com/plugin.orax)";
        assert_eq!(
            ReleaseUrl::parse(release).expect("release").as_str(),
            "https://example.com/plugin.orax"
        );

        // Plain URLs pass through unchanged.
        assert!(ReleaseUrl::parse("https://example.com/plugin.orax").is_ok());
    }

    /// Verifies the shared URL byte limit accepts its boundary and rejects one byte above it.
    #[test]
    fn enforces_url_byte_limit() {
        let prefix = "https://example.com/";
        let boundary = format!("{prefix}{}", "a".repeat(2048 - prefix.len()));
        let over_limit = format!("{boundary}a");

        assert!(ReleaseUrl::parse(&boundary).is_ok());
        assert!(matches!(
            ReleaseUrl::parse(&over_limit),
            Err(UrlError::TooLong {
                max_bytes: 2048,
                actual_bytes: 2049,
            })
        ));
    }
}

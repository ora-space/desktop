use crate::{InvalidFieldReason, ManifestError, ManifestField, RuleField};
use serde::Deserialize;
use std::{fmt, str::FromStr};
use thiserror::Error;
use url::Url;

/// Upper bound on one origin or start URL; matches the shared manifest URL limit.
const MAX_URL_BYTES: usize = 2048;
/// Upper bound on one page path prefix; long enough for deep routes, short enough to log.
const MAX_PATH_PREFIX_BYTES: usize = 1024;

/// Holds the validated `[webview]` section of a webview-kind plugin manifest.
///
/// The manifest crate guarantees shape and per-value syntax: an HTTPS start URL without
/// credentials or fragment, well-formed HTTPS origins, well-formed page matchers and actions.
/// Cross-value policy that needs the whole declaration (origin duplicates, start URL coverage,
/// shadowed rules) is applied by the plugin manager, which reports it against the package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginWebview {
    pub(crate) start_url: StartUrl,
    pub(crate) allowed_origins: Vec<Origin>,
    pub(crate) downloads: DownloadPolicy,
}

impl PluginWebview {
    /// Returns the page the webview opens on.
    pub fn start_url(&self) -> &StartUrl {
        &self.start_url
    }

    /// Returns the declared origins in manifest order; there is always at least one.
    pub fn allowed_origins(&self) -> &[Origin] {
        &self.allowed_origins
    }

    /// Returns the download rules and fallback.
    pub fn downloads(&self) -> &DownloadPolicy {
        &self.downloads
    }
}

/// An absolute HTTPS URL without credentials or fragment; a query is allowed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartUrl(Url);

impl StartUrl {
    /// Parses the start URL; the origin it belongs to is what `allowed_origins` must cover.
    pub fn parse(value: &str) -> Result<Self, WebviewUrlError> {
        let parsed = parse_https(value)?;
        if parsed.fragment().is_some() {
            return Err(WebviewUrlError::FragmentNotAllowed);
        }

        Ok(Self(parsed))
    }

    /// Returns the parsed URL.
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// Returns the normalized origin of this URL.
    pub fn origin(&self) -> Origin {
        Origin(self.0.origin().ascii_serialization())
    }
}

impl fmt::Display for StartUrl {
    /// Writes the normalized URL.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// A normalized HTTPS origin: `https://host` or `https://host:port` for a non-default port.
///
/// Origins are the unit of navigation trust for a webview plugin. They are exact: no wildcard,
/// no registrable-domain suffix, no path. Normalization (lowercase host, dropped default port)
/// happens at parse time so equality and matching never depend on manifest spelling.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Origin(String);

impl Origin {
    /// Parses one origin, rejecting anything beyond scheme, host, and an explicit port.
    pub fn parse(value: &str) -> Result<Self, WebviewUrlError> {
        let parsed = parse_https(value)?;
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(WebviewUrlError::OriginMustBeBare);
        }
        // The URL parser turns `https://host` into `https://host/`; anything longer is a path.
        if parsed.path() != "/" {
            return Err(WebviewUrlError::OriginMustBeBare);
        }

        Ok(Self(parsed.origin().ascii_serialization()))
    }

    /// Reports whether `url` belongs to this origin (scheme, host, and effective port).
    pub fn matches(&self, url: &Url) -> bool {
        url.origin().ascii_serialization() == self.0
    }

    /// Returns the normalized origin serialization.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Origin {
    /// Writes the normalized origin.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Origin {
    type Err = WebviewUrlError;

    /// Parses an origin through the same rules as `parse`.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Applies the HTTPS, credential, and length rules shared by start URLs and origins.
fn parse_https(value: &str) -> Result<Url, WebviewUrlError> {
    if value.len() > MAX_URL_BYTES {
        return Err(WebviewUrlError::TooLong {
            max_bytes: MAX_URL_BYTES,
            actual_bytes: value.len(),
        });
    }
    let parsed = Url::parse(value).map_err(WebviewUrlError::InvalidSyntax)?;
    if parsed.scheme() != "https" {
        return Err(WebviewUrlError::NotHttps);
    }
    if parsed.host_str().is_none() {
        return Err(WebviewUrlError::MissingHost);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(WebviewUrlError::CredentialsNotAllowed);
    }

    Ok(parsed)
}

/// Reports why a webview URL or origin was rejected.
#[derive(Debug, Error)]
pub enum WebviewUrlError {
    #[error("URL exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("URL syntax is invalid: {0}")]
    InvalidSyntax(#[source] url::ParseError),
    #[error("URL scheme must be HTTPS")]
    NotHttps,
    #[error("URL must have a host")]
    MissingHost,
    #[error("URL must not contain a username or password")]
    CredentialsNotAllowed,
    #[error("URL must not contain a fragment")]
    FragmentNotAllowed,
    #[error("origin must contain only a scheme, host, and optional port")]
    OriginMustBeBare,
}

/// The ordered download rules of one webview plugin plus what happens when none match.
///
/// Rules are consulted in declaration order and the first match decides; `fallback` applies
/// when none match and is `Reject` unless declared otherwise.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadPolicy {
    pub rules: Vec<DownloadRule>,
    pub fallback: DownloadDisposition,
}

impl Default for DownloadPolicy {
    /// Omitting `[webview.downloads]` rejects every download.
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            fallback: DownloadDisposition::Reject,
        }
    }
}

/// One `[[webview.downloads.rules]]` entry: a page matcher and the disposition it selects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadRule {
    pub page: PageMatcher,
    pub disposition: DownloadDisposition,
}

/// Matches the main-frame page URL that initiated a download.
///
/// Only the origin and a path prefix take part; query and fragment never do, so a rule can
/// neither depend on nor leak a token. The prefix is compared against the URL path after
/// normalization and one round of valid percent-decoding, which is also how it is stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageMatcher {
    pub origin: Origin,
    pub path_prefix: PathPrefix,
}

impl PageMatcher {
    /// Reports whether `page_url` is on this origin and its decoded path starts with the prefix.
    pub fn matches(&self, page_url: &Url) -> bool {
        self.origin.matches(page_url)
            && percent_decode_once(page_url.path())
                .is_some_and(|path| path.starts_with(self.path_prefix.as_str()))
    }
}

/// A decoded URL path prefix starting with `/`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathPrefix(String);

impl PathPrefix {
    /// Parses one prefix: rooted, without query, fragment, control characters, or invalid
    /// percent escapes; stored decoded once so matching compares like with like.
    pub fn parse(value: &str) -> Result<Self, PathPrefixError> {
        if !value.starts_with('/') {
            return Err(PathPrefixError::NotRooted);
        }
        if value.len() > MAX_PATH_PREFIX_BYTES {
            return Err(PathPrefixError::TooLong {
                max_bytes: MAX_PATH_PREFIX_BYTES,
                actual_bytes: value.len(),
            });
        }
        if value.contains(['?', '#']) {
            return Err(PathPrefixError::ContainsQueryOrFragment);
        }
        if value.chars().any(char::is_control) {
            return Err(PathPrefixError::ContainsControlCharacter);
        }
        let decoded = percent_decode_once(value).ok_or(PathPrefixError::InvalidPercentEncoding)?;

        Ok(Self(decoded))
    }

    /// Returns the decoded prefix.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reports why a page path prefix was rejected.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PathPrefixError {
    #[error("path prefix must start with `/`")]
    NotRooted,
    #[error("path prefix exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("path prefix must not contain a query or fragment")]
    ContainsQueryOrFragment,
    #[error("path prefix must not contain control characters")]
    ContainsControlCharacter,
    #[error("path prefix contains an invalid percent escape")]
    InvalidPercentEncoding,
}

/// Decodes `%XX` escapes exactly once, failing on a malformed escape or non-UTF-8 result.
///
/// Decoding once (never repeatedly) is what keeps `%252F` from becoming `/` on the second pass;
/// both the rule and the page path go through this same function.
fn percent_decode_once(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let escape = bytes.get(index + 1..index + 3)?;
            let text = std::str::from_utf8(escape).ok()?;
            decoded.push(u8::from_str_radix(text, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).ok()
}

/// What the host does with a download the rule (or fallback) applies to.
///
/// The enum is exclusive by construction: a rule either runs one action, offers a choice, or
/// rejects. `Prompt` carries a non-empty, duplicate-free action list in declaration order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadDisposition {
    Auto { action: DownloadAction },
    Prompt { actions: Vec<DownloadAction> },
    Reject,
}

/// The closed set of host-owned download actions a webview plugin may select.
///
/// These are host capabilities, not plugin-registered strings: adding one means extending the
/// host implementation, the install review, and this enum together.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DownloadAction {
    ImportSkill,
    SaveAs,
}

impl DownloadAction {
    /// Returns the resolver-1 manifest spelling of this action.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImportSkill => "import_skill",
            Self::SaveAs => "save_as",
        }
    }
}

impl fmt::Display for DownloadAction {
    /// Writes the manifest spelling.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DownloadAction {
    type Err = DownloadActionError;

    /// Parses an action without accepting unknown spellings.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "import_skill" => Ok(Self::ImportSkill),
            "save_as" => Ok(Self::SaveAs),
            found => Err(DownloadActionError::Unknown {
                found: found.to_owned(),
            }),
        }
    }
}

/// Reports an action the host does not implement.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DownloadActionError {
    #[error("unknown download action {found:?}")]
    Unknown { found: String },
}

/// Mirrors `[webview]` before semantic validation; unknown fields fail structurally.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWebview {
    start_url: String,
    allowed_origins: Vec<String>,
    downloads: Option<RawDownloads>,
}

/// Mirrors `[webview.downloads]`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawDownloads {
    fallback: Option<RawAction>,
    #[serde(default)]
    rules: Vec<RawRule>,
}

/// Mirrors one `[[webview.downloads.rules]]` entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawRule {
    page: RawPage,
    action: RawAction,
}

/// Mirrors `rules[].page`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawPage {
    origin: String,
    path_prefix: String,
}

/// Mirrors an action table: `{ auto = "…" }`, `{ prompt = ["…"] }`, or `{ reject = true }`.
///
/// Exclusivity is a semantic rule (so the error names the rule index) rather than a serde
/// untagged enum, whose failure would only say the whole table did not match any variant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawAction {
    auto: Option<String>,
    prompt: Option<Vec<String>>,
    reject: Option<bool>,
}

impl TryFrom<RawWebview> for PluginWebview {
    type Error = ManifestError;

    /// Validates every value in declaration order so the first error is deterministic.
    fn try_from(raw: RawWebview) -> Result<Self, Self::Error> {
        let start_url =
            StartUrl::parse(&raw.start_url).map_err(|reason| ManifestError::InvalidField {
                field: ManifestField::WebviewStartUrl,
                reason: reason.into(),
            })?;
        if raw.allowed_origins.is_empty() {
            return Err(ManifestError::InvalidField {
                field: ManifestField::WebviewAllowedOrigins,
                reason: InvalidFieldReason::Empty,
            });
        }
        let allowed_origins = raw
            .allowed_origins
            .iter()
            .enumerate()
            .map(|(index, value)| {
                Origin::parse(value).map_err(|reason| ManifestError::InvalidField {
                    field: ManifestField::WebviewAllowedOrigin { index },
                    reason: reason.into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let downloads = raw
            .downloads
            .map(DownloadPolicy::try_from)
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            start_url,
            allowed_origins,
            downloads,
        })
    }
}

impl TryFrom<RawDownloads> for DownloadPolicy {
    type Error = ManifestError;

    /// Validates the fallback and every rule, attributing failures to their indexed field.
    fn try_from(raw: RawDownloads) -> Result<Self, Self::Error> {
        let fallback = raw
            .fallback
            .map(|action| disposition(action, ManifestField::WebviewDownloadsFallback))
            .transpose()?
            .unwrap_or(DownloadDisposition::Reject);
        let rules = raw
            .rules
            .into_iter()
            .enumerate()
            .map(|(index, rule)| {
                let invalid =
                    |field: RuleField, reason: InvalidFieldReason| ManifestError::InvalidField {
                        field: ManifestField::WebviewDownloadRule { index, field },
                        reason,
                    };
                let origin = Origin::parse(&rule.page.origin)
                    .map_err(|reason| invalid(RuleField::PageOrigin, reason.into()))?;
                let path_prefix = PathPrefix::parse(&rule.page.path_prefix)
                    .map_err(|reason| invalid(RuleField::PagePathPrefix, reason.into()))?;
                let disposition = disposition(
                    rule.action,
                    ManifestField::WebviewDownloadRule {
                        index,
                        field: RuleField::Action,
                    },
                )?;
                Ok(DownloadRule {
                    page: PageMatcher {
                        origin,
                        path_prefix,
                    },
                    disposition,
                })
            })
            .collect::<Result<Vec<_>, ManifestError>>()?;

        Ok(Self { rules, fallback })
    }
}

/// Converts one action table into a disposition, requiring exactly one of its forms.
fn disposition(raw: RawAction, field: ManifestField) -> Result<DownloadDisposition, ManifestError> {
    let invalid = |reason: InvalidFieldReason| ManifestError::InvalidField { field, reason };
    match (raw.auto, raw.prompt, raw.reject) {
        (Some(action), None, None) => {
            let action: DownloadAction = action
                .parse()
                .map_err(|reason| invalid(InvalidFieldReason::InvalidDownloadAction(reason)))?;
            // An automatic disposition runs without any user in the loop, so only actions the
            // host can complete on its own qualify; `save_as` needs a user-chosen destination.
            match action {
                DownloadAction::ImportSkill => {}
                DownloadAction::SaveAs => {
                    return Err(invalid(InvalidFieldReason::NonAutomatableDownloadAction {
                        action: action.as_str().to_owned(),
                    }));
                }
            }
            Ok(DownloadDisposition::Auto { action })
        }
        (None, Some(actions), None) => {
            if actions.is_empty() {
                return Err(invalid(InvalidFieldReason::Empty));
            }
            let mut parsed = Vec::with_capacity(actions.len());
            for action in &actions {
                let action = action
                    .parse::<DownloadAction>()
                    .map_err(|reason| invalid(InvalidFieldReason::InvalidDownloadAction(reason)))?;
                if parsed.contains(&action) {
                    return Err(invalid(InvalidFieldReason::Duplicate));
                }
                parsed.push(action);
            }
            Ok(DownloadDisposition::Prompt { actions: parsed })
        }
        (None, None, Some(true)) => Ok(DownloadDisposition::Reject),
        (None, None, Some(false)) | (None, None, None) => {
            Err(invalid(InvalidFieldReason::AmbiguousDownloadAction))
        }
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
            Err(invalid(InvalidFieldReason::AmbiguousDownloadAction))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Origin, PageMatcher, PathPrefix, PathPrefixError, StartUrl, WebviewUrlError};
    use pretty_assertions::assert_eq;
    use url::Url;

    /// Origins normalize the host and drop the default port so spelling never affects matching.
    #[test]
    fn origins_normalize_and_match_exactly() {
        let origin = Origin::parse("https://WWW.Example.com:443").expect("origin");
        let url = |value: &str| Url::parse(value).expect("url");

        assert_eq!(origin.as_str(), "https://www.example.com");
        assert_eq!(
            (
                origin.matches(&url("https://www.example.com/skills/1?x=1")),
                origin.matches(&url("https://example.com/")),
                origin.matches(&url("https://www.example.com:8443/")),
                origin.matches(&url("http://www.example.com/")),
            ),
            (true, false, false, false)
        );
    }

    /// Anything beyond scheme, host, and port is not an origin.
    #[test]
    fn rejects_non_bare_origins_and_bad_urls() {
        assert!(matches!(
            Origin::parse("https://example.com/skills"),
            Err(WebviewUrlError::OriginMustBeBare)
        ));
        assert!(matches!(
            Origin::parse("https://example.com/?q=1"),
            Err(WebviewUrlError::OriginMustBeBare)
        ));
        assert!(matches!(
            Origin::parse("http://example.com"),
            Err(WebviewUrlError::NotHttps)
        ));
        assert!(matches!(
            Origin::parse("https://user:pw@example.com"),
            Err(WebviewUrlError::CredentialsNotAllowed)
        ));
        assert!(matches!(
            StartUrl::parse("https://example.com/#top"),
            Err(WebviewUrlError::FragmentNotAllowed)
        ));
        assert!(StartUrl::parse("https://example.com/skills?sort=new").is_ok());
    }

    /// Prefixes decode once; the page path decodes once too, so `%2F` never becomes a separator
    /// on a second pass, and query or fragment never take part.
    #[test]
    fn path_prefixes_match_decoded_paths_only() {
        let matcher = PageMatcher {
            origin: Origin::parse("https://www.example.com").expect("origin"),
            path_prefix: PathPrefix::parse("/skills/").expect("prefix"),
        };
        let url = |value: &str| Url::parse(value).expect("url");

        assert_eq!(
            (
                matcher.matches(&url("https://www.example.com/skills/42?download=1#x")),
                matcher.matches(&url("https://www.example.com/skills")),
                matcher.matches(&url("https://www.example.com/skills%2F42")),
                matcher.matches(&url("https://example.com/skills/42")),
            ),
            (true, false, true, false)
        );
        assert_eq!(
            PathPrefix::parse("/a%20b").expect("prefix").as_str(),
            "/a b"
        );
        assert!(matches!(
            PathPrefix::parse("skills/"),
            Err(PathPrefixError::NotRooted)
        ));
        assert!(matches!(
            PathPrefix::parse("/skills/?x"),
            Err(PathPrefixError::ContainsQueryOrFragment)
        ));
        assert!(matches!(
            PathPrefix::parse("/skills/%zz"),
            Err(PathPrefixError::InvalidPercentEncoding)
        ));
    }
}

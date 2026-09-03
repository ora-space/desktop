//! Release locators for marketplace `url` fields: an HTTPS download or an S3 object key.

use crate::urls::{ReleaseUrl, UrlError, strip_markdown_link};
use std::fmt;
use thiserror::Error;

/// Maximum accepted object-key length; S3 allows more, but marketplace keys stay well below this.
const MAX_OBJECT_KEY_BYTES: usize = 1024;

/// Where a marketplace release archive can be fetched from.
///
/// HTTPS URLs are fetched verbatim. Object keys are resolved against the host's configured
/// object-store endpoint at install time, so a listing can name a package without baking a
/// signed URL into Git.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseLocator {
    /// A credential-free HTTPS URL, optionally carrying a signing query.
    Https(ReleaseUrl),
    /// An object-store key resolved by the installer against a configured bucket.
    ObjectKey(ObjectKey),
}

impl ReleaseLocator {
    /// Parses a marketplace `url` value as HTTPS when it uses that scheme, otherwise as an object key.
    ///
    /// Non-HTTPS schemes (`http://`, `s3://`, …) are rejected rather than being mistaken for keys.
    pub fn parse(value: &str) -> Result<Self, ReleaseLocatorError> {
        let unwrapped = strip_markdown_link(value);
        if unwrapped.starts_with("https://") {
            return Ok(Self::Https(ReleaseUrl::parse(value)?));
        }
        // Non-HTTPS schemes must not be mistaken for object keys; reuse URL validation so
        // `http://` reports NotHttps instead of an object-key error.
        if unwrapped.contains("://") {
            return match ReleaseUrl::parse(value) {
                Ok(_) => {
                    unreachable!("a locator containing a non-https scheme cannot parse as HTTPS")
                }
                Err(error) => Err(error.into()),
            };
        }
        Ok(Self::ObjectKey(ObjectKey::parse(unwrapped)?))
    }

    /// Returns the locator as the original HTTPS URL or object-key string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Https(url) => url.as_str(),
            Self::ObjectKey(key) => key.as_str(),
        }
    }
}

impl fmt::Display for ReleaseLocator {
    /// Writes the HTTPS URL or object key.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Holds a validated object-store key used as a marketplace release locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// Parses an object key, rejecting traversal, schemes, and absolute paths.
    pub fn parse(value: &str) -> Result<Self, ObjectKeyError> {
        if value.is_empty() {
            return Err(ObjectKeyError::Empty);
        }
        if value.len() > MAX_OBJECT_KEY_BYTES {
            return Err(ObjectKeyError::TooLong {
                max_bytes: MAX_OBJECT_KEY_BYTES,
                actual_bytes: value.len(),
            });
        }
        if value.chars().next().is_some_and(char::is_whitespace)
            || value.chars().next_back().is_some_and(char::is_whitespace)
        {
            return Err(ObjectKeyError::LeadingOrTrailingWhitespace);
        }
        if value.chars().any(char::is_control) {
            return Err(ObjectKeyError::ContainsControlCharacter);
        }
        if value.contains("://") {
            return Err(ObjectKeyError::SchemeNotAllowed);
        }
        if value.starts_with('/') || value.starts_with('\\') {
            return Err(ObjectKeyError::Absolute);
        }
        if value.split(['/', '\\']).any(|segment| segment == "..") {
            return Err(ObjectKeyError::ParentSegment);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the key as written in the manifest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectKey {
    /// Writes the object key.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Describes why a marketplace `url` field could not be parsed as a locator.
#[derive(Debug, Error)]
pub enum ReleaseLocatorError {
    #[error(transparent)]
    Url(#[from] UrlError),
    #[error(transparent)]
    ObjectKey(#[from] ObjectKeyError),
}

/// Describes why an object-store key was rejected.
#[derive(Debug, Error)]
pub enum ObjectKeyError {
    #[error("field must not be empty")]
    Empty,
    #[error("field exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("field must not contain leading or trailing whitespace")]
    LeadingOrTrailingWhitespace,
    #[error("field must not contain control characters")]
    ContainsControlCharacter,
    #[error("object key must not contain a URL scheme")]
    SchemeNotAllowed,
    #[error("object key must not be an absolute path")]
    Absolute,
    #[error("object key must not contain a parent-directory segment")]
    ParentSegment,
}

#[cfg(test)]
mod tests {
    use super::{ObjectKey, ObjectKeyError, ReleaseLocator, ReleaseLocatorError};
    use crate::urls::UrlError;
    use pretty_assertions::assert_eq;

    /// HTTPS release URLs stay HTTPS locators, including Markdown wrappers.
    #[test]
    fn parses_https_urls_as_https_locators() {
        let locator = ReleaseLocator::parse("https://example.com/plugin.orax").expect("https");
        assert_eq!(locator.as_str(), "https://example.com/plugin.orax");
        assert!(matches!(locator, ReleaseLocator::Https(_)));

        let wrapped = ReleaseLocator::parse(
            "[https://example.com/plugin.orax](https://example.com/plugin.orax)",
        )
        .expect("markdown https");
        assert_eq!(wrapped.as_str(), "https://example.com/plugin.orax");
    }

    /// A bare object key is accepted and round-trips.
    #[test]
    fn parses_object_keys() {
        let locator = ReleaseLocator::parse("ora-space.opencode-v0.1.3.orax").expect("object key");
        assert_eq!(locator.as_str(), "ora-space.opencode-v0.1.3.orax");
        assert!(matches!(locator, ReleaseLocator::ObjectKey(_)));

        let nested = ReleaseLocator::parse("plugins/ora-space.opencode-v0.1.3.orax")
            .expect("nested object key");
        assert_eq!(nested.as_str(), "plugins/ora-space.opencode-v0.1.3.orax");
    }

    /// `http://` and other schemes are rejected instead of being treated as keys.
    #[test]
    fn rejects_non_https_schemes() {
        assert!(matches!(
            ReleaseLocator::parse("http://example.com/plugin.orax"),
            Err(ReleaseLocatorError::Url(UrlError::NotHttps))
        ));
        assert!(matches!(
            ReleaseLocator::parse("s3://bucket/plugin.orax"),
            Err(ReleaseLocatorError::Url(UrlError::NotHttps))
        ));
    }

    /// Object-key validation rejects empty, absolute, traversing, and oversized values.
    #[test]
    fn rejects_invalid_object_keys() {
        assert!(matches!(ObjectKey::parse(""), Err(ObjectKeyError::Empty)));
        assert!(matches!(
            ObjectKey::parse("/absolute.orax"),
            Err(ObjectKeyError::Absolute)
        ));
        assert!(matches!(
            ObjectKey::parse("plugins/../secret.orax"),
            Err(ObjectKeyError::ParentSegment)
        ));
        assert!(matches!(
            ObjectKey::parse(" leading.orax"),
            Err(ObjectKeyError::LeadingOrTrailingWhitespace)
        ));
        let too_long = "a".repeat(1025);
        assert!(matches!(
            ObjectKey::parse(&too_long),
            Err(ObjectKeyError::TooLong {
                max_bytes: 1024,
                actual_bytes: 1025
            })
        ));
    }
}

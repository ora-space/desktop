use ora_utils::Slug;
use std::fmt;
use thiserror::Error;

/// The longest surface identifier accepted by the manifest.
///
/// Surface ids are combined with the plugin id into webview labels and on-disk profile directory
/// names, so they stay well below the generic slug limit.
const MAX_SURFACE_ID_BYTES: usize = 32;
/// The longest DNS name accepted in navigation allow lists (RFC 1035 wire limit).
const MAX_HOST_NAME_BYTES: usize = 253;
/// The longest single DNS label (RFC 1035).
const MAX_HOST_LABEL_BYTES: usize = 63;

/// Identifies one surface inside its plugin package.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(Slug);

impl SurfaceId {
    /// Parses one surface id, rejecting slugs longer than the surface-specific limit.
    pub fn parse(value: &str) -> Result<Self, SurfaceIdError> {
        let slug = Slug::parse(value)?;
        if slug.as_str().len() > MAX_SURFACE_ID_BYTES {
            return Err(SurfaceIdError::TooLong {
                max_bytes: MAX_SURFACE_ID_BYTES,
                actual_bytes: slug.as_str().len(),
            });
        }
        Ok(Self(slug))
    }

    /// Returns the validated id spelling.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Describes why one string cannot be represented as a [`SurfaceId`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SurfaceIdError {
    #[error(transparent)]
    Slug(#[from] ora_utils::SlugError),
    #[error("surface id exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
}

/// Holds one lowercase ASCII DNS name used by navigation allow lists.
///
/// The type deliberately refuses to normalize: a manifest author who writes `Example.com` or
/// `https://example.com` gets a validation error instead of a silently rewritten policy.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HostName(String);

impl HostName {
    /// Parses one DNS name without scheme, port, path, or uppercase letters.
    pub fn parse(value: &str) -> Result<Self, HostNameError> {
        if value.is_empty() {
            return Err(HostNameError::Empty);
        }
        if value.len() > MAX_HOST_NAME_BYTES {
            return Err(HostNameError::TooLong {
                max_bytes: MAX_HOST_NAME_BYTES,
                actual_bytes: value.len(),
            });
        }
        for label in value.split('.') {
            if label.is_empty() {
                return Err(HostNameError::EmptyLabel);
            }
            if label.len() > MAX_HOST_LABEL_BYTES {
                return Err(HostNameError::LabelTooLong {
                    max_bytes: MAX_HOST_LABEL_BYTES,
                    label: label.to_owned(),
                });
            }
            if label.starts_with('-') || label.ends_with('-') {
                return Err(HostNameError::LabelHyphenEdge {
                    label: label.to_owned(),
                });
            }
            // Any other character (uppercase, `:`, `/`, `@`, unicode) is rejected here, which is
            // what keeps schemes, ports, paths, and credentials out of an allow list entry.
            if let Some(character) = label
                .chars()
                .find(|c| !c.is_ascii_lowercase() && !c.is_ascii_digit() && *c != '-')
            {
                return Err(HostNameError::InvalidCharacter { character });
            }
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated host spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reports whether `host` equals this name or lives in a subdomain of it.
    pub fn matches_suffix_of(&self, host: &str) -> bool {
        host == self.0
            || host
                .strip_suffix(self.0.as_str())
                .is_some_and(|prefix| prefix.ends_with('.'))
    }
}

impl fmt::Display for HostName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Describes why one string cannot be represented as a [`HostName`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HostNameError {
    #[error("host must not be empty")]
    Empty,
    #[error("host exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("host must not contain an empty label")]
    EmptyLabel,
    #[error("host label `{label}` exceeds {max_bytes} bytes")]
    LabelTooLong { max_bytes: usize, label: String },
    #[error("host label `{label}` must not start or end with a hyphen")]
    LabelHyphenEdge { label: String },
    #[error("host contains invalid character {character:?}; expected a lowercase ASCII DNS name")]
    InvalidCharacter { character: char },
}

/// Declares how many live instances of one surface the host may open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstancePolicy {
    Singleton,
}

/// Declares where the web data (cookies, storage) of one remote site surface lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebDataPolicy {
    PersistentProfile,
    EphemeralIsolated,
}

#[cfg(test)]
mod tests {
    use super::{HostName, HostNameError, SurfaceId, SurfaceIdError};
    use ora_utils::SlugError;
    use pretty_assertions::assert_eq;

    /// Verifies surface ids follow slug rules plus the tighter surface length limit.
    #[test]
    fn surface_id_table() {
        let cases = [
            ("market", Ok(())),
            ("a", Ok(())),
            (&"a".repeat(32), Ok(())),
            (
                &"a".repeat(33),
                Err(SurfaceIdError::TooLong {
                    max_bytes: 32,
                    actual_bytes: 33,
                }),
            ),
            ("", Err(SurfaceIdError::Slug(SlugError::Empty))),
            (
                "Market",
                Err(SurfaceIdError::Slug(SlugError::InvalidCharacter {
                    index: 0,
                    character: 'M',
                })),
            ),
            (
                "-market",
                Err(SurfaceIdError::Slug(SlugError::LeadingHyphen)),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                SurfaceId::parse(input).map(|id| assert_eq!(id.as_str(), input)),
                expected,
                "{input}"
            );
        }
    }

    /// Verifies host names accept DNS syntax only and reject URL-shaped input.
    #[test]
    fn host_name_table() {
        let long_label = "a".repeat(64);
        let long_host = format!("{}.com", "a".repeat(250));
        let cases = [
            ("skillhub.cn", Ok(())),
            ("www.skillhub.cn", Ok(())),
            ("localhost", Ok(())),
            ("x-1.example-site.com", Ok(())),
            ("", Err(HostNameError::Empty)),
            (
                "Example.com",
                Err(HostNameError::InvalidCharacter { character: 'E' }),
            ),
            (
                "https://example.com",
                Err(HostNameError::InvalidCharacter { character: ':' }),
            ),
            (
                "example.com:443",
                Err(HostNameError::InvalidCharacter { character: ':' }),
            ),
            (
                "example.com/path",
                Err(HostNameError::InvalidCharacter { character: '/' }),
            ),
            ("example..com", Err(HostNameError::EmptyLabel)),
            (".example.com", Err(HostNameError::EmptyLabel)),
            (
                "-bad.example.com",
                Err(HostNameError::LabelHyphenEdge {
                    label: "-bad".to_string(),
                }),
            ),
            (
                long_label.as_str(),
                Err(HostNameError::LabelTooLong {
                    max_bytes: 63,
                    label: long_label.clone(),
                }),
            ),
            (
                long_host.as_str(),
                Err(HostNameError::TooLong {
                    max_bytes: 253,
                    actual_bytes: 254,
                }),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                HostName::parse(input).map(|host| assert_eq!(host.as_str(), input)),
                expected,
                "{input}"
            );
        }
    }

    /// Verifies suffix matching respects label boundaries.
    #[test]
    fn host_suffix_matching_table() {
        let suffix = HostName::parse("huawei.com").unwrap();
        let cases = [
            ("huawei.com", true),
            ("www.huawei.com", true),
            ("a.b.huawei.com", true),
            ("nothuawei.com", false),
            ("huawei.com.evil", false),
            ("huawei.cn", false),
        ];
        for (host, expected) in cases {
            assert_eq!(suffix.matches_suffix_of(host), expected, "{host}");
        }
    }
}

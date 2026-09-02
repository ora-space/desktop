use crate::plugin_namespace::MAX_PLUGIN_NAMESPACE_BYTES;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{self, Display, Formatter};
use thiserror::Error;

/// Separator between the namespace and name segments of a canonical plugin id.
const CANONICAL_SEPARATOR: char = '/';

/// Identifies one installed plugin across lifecycle layers as `<namespace>/<name>`.
///
/// The two segments are kept apart instead of storing the joined string because directory
/// layouts (`installed/<namespace>/<name>/`, `data/<namespace>/<name>/`) and webview labels
/// need them separately; the canonical string is only the wire and persistence spelling.
///
/// This type enforces the structural shape only: both segments are non-empty, are limited to
/// lowercase ASCII letters, digits, `-`, and `.`, and are never `.` or `..`. The namespace
/// segment carries the additional length bound of [`crate::PluginNamespace`], because it is the
/// namespace — not the name — that the host generates and must keep inside a bounded directory
/// level. The full slug grammar of the name segment (segment count, hyphen placement, length
/// budgets) is owned by manifest validation in `ora-plugin-manager`, which is the only place a
/// plugin name is first admitted into the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId {
    namespace: String,
    name: String,
}

impl PluginId {
    /// Builds an id from already validated segments.
    ///
    /// Callers that hold untrusted text must go through [`PluginId::parse`] instead; this
    /// constructor still rejects structurally impossible segments so a malformed id can never be
    /// rendered into a path or label.
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, PluginIdError> {
        let namespace = namespace.into();
        let name = name.into();
        validate_segment(&namespace, PluginIdSegment::Namespace)?;
        validate_segment(&name, PluginIdSegment::Name)?;
        Ok(Self { namespace, name })
    }

    /// Parses the canonical `<namespace>/<name>` spelling.
    pub fn parse(value: &str) -> Result<Self, PluginIdError> {
        let Some((namespace, name)) = value.split_once(CANONICAL_SEPARATOR) else {
            return Err(PluginIdError::MissingSeparator);
        };
        Self::new(namespace, name)
    }

    /// Returns the namespace segment, e.g. `official`.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the name segment, e.g. `acme.hub`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the canonical `<namespace>/<name>` spelling used on the wire and in storage.
    pub fn canonical(&self) -> String {
        format!("{}{CANONICAL_SEPARATOR}{}", self.namespace, self.name)
    }
}

impl Display for PluginId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}{CANONICAL_SEPARATOR}{}",
            self.namespace, self.name
        )
    }
}

impl Serialize for PluginId {
    /// Serializes the canonical spelling so JSON consumers see one opaque string.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> Deserialize<'de> for PluginId {
    /// Deserializes the canonical spelling, rejecting anything that is not a valid id.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

/// Names the segment a structural error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginIdSegment {
    Namespace,
    Name,
}

impl Display for PluginIdSegment {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Namespace => "namespace",
            Self::Name => "name",
        })
    }
}

/// Describes why a string cannot be represented as a [`PluginId`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PluginIdError {
    #[error("plugin id must be spelled `<namespace>/<name>`")]
    MissingSeparator,
    #[error("plugin id {segment} must not be empty")]
    EmptySegment { segment: PluginIdSegment },
    #[error("plugin id {segment} contains invalid character {character:?}")]
    InvalidCharacter {
        segment: PluginIdSegment,
        character: char,
    },
    #[error("plugin id {segment} must not exceed {max_bytes} bytes, found {actual_bytes}")]
    SegmentTooLong {
        segment: PluginIdSegment,
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("plugin id {segment} must not be `.` or `..`")]
    RelativeSegment { segment: PluginIdSegment },
}

/// Rejects empty segments, any character outside the lowercase slug alphabet plus `.`, the two
/// relative path spellings, and — for the namespace only — anything past the namespace length
/// bound.
///
/// Both segments become directory levels, so `.` and `..` are refused here rather than at each
/// path-building call site: a segment that traverses upward would place a package's files outside
/// the tree the host owns.
pub(crate) fn validate_segment(value: &str, segment: PluginIdSegment) -> Result<(), PluginIdError> {
    if value.is_empty() {
        return Err(PluginIdError::EmptySegment { segment });
    }
    // `/` is excluded implicitly: it is not in the allowed set, which keeps `canonical()` and
    // `parse()` inverse of each other and keeps ids single path components.
    if let Some(character) = value
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.')))
    {
        return Err(PluginIdError::InvalidCharacter { segment, character });
    }
    if matches!(value, "." | "..") {
        return Err(PluginIdError::RelativeSegment { segment });
    }
    // Only the namespace is bounded: it is host-generated and its width decides how long an
    // installed package's first directory level can get, while the name segment's budget belongs
    // to the manifest grammar that admits it.
    if matches!(segment, PluginIdSegment::Namespace) && value.len() > MAX_PLUGIN_NAMESPACE_BYTES {
        return Err(PluginIdError::SegmentTooLong {
            segment,
            max_bytes: MAX_PLUGIN_NAMESPACE_BYTES,
            actual_bytes: value.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PluginId, PluginIdError, PluginIdSegment};
    use pretty_assertions::assert_eq;

    /// Verifies parse and canonical are inverse and the accessors expose the segments.
    #[test]
    fn parses_canonical_form() {
        let id = PluginId::parse("official/acme.hub").expect("valid id");
        assert_eq!(
            (
                id.namespace(),
                id.name(),
                id.canonical(),
                id.to_string(),
                PluginId::new("official", "acme.hub"),
            ),
            (
                "official",
                "acme.hub",
                "official/acme.hub".to_string(),
                "official/acme.hub".to_string(),
                Ok(id.clone()),
            )
        );
    }

    /// Verifies structural rejections report the offending segment.
    #[test]
    fn rejects_malformed_ids() {
        let cases = [
            ("acme.hub", Err(PluginIdError::MissingSeparator)),
            (
                "/example",
                Err(PluginIdError::EmptySegment {
                    segment: PluginIdSegment::Namespace,
                }),
            ),
            (
                "official/",
                Err(PluginIdError::EmptySegment {
                    segment: PluginIdSegment::Name,
                }),
            ),
            (
                "official/a/b",
                Err(PluginIdError::InvalidCharacter {
                    segment: PluginIdSegment::Name,
                    character: '/',
                }),
            ),
            (
                "Official/example",
                Err(PluginIdError::InvalidCharacter {
                    segment: PluginIdSegment::Namespace,
                    character: 'O',
                }),
            ),
            (
                "official/skill_hub",
                Err(PluginIdError::InvalidCharacter {
                    segment: PluginIdSegment::Name,
                    character: '_',
                }),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(PluginId::parse(input), expected, "{input}");
        }
    }

    /// Verifies serde uses the canonical string and refuses malformed input.
    #[test]
    fn serde_round_trips_canonical_string() {
        let id = PluginId::parse("official/acme.hub").expect("valid id");
        assert_eq!(
            (
                serde_json::to_string(&id).expect("serialize"),
                serde_json::from_str::<PluginId>("\"official/acme.hub\"").ok(),
                serde_json::from_str::<PluginId>("\"acme.hub\"").is_err(),
            ),
            ("\"official/acme.hub\"".to_string(), Some(id), true)
        );
    }
}

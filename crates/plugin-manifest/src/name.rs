use ora_utils::{Slug, SlugError};
use std::{fmt, str::FromStr};
use thiserror::Error;

const MAX_PLUGIN_NAME_BYTES: usize = 128;
const MAX_PLUGIN_NAME_SEGMENTS: usize = 2;

/// Holds a complete plugin identifier composed of one or two validated slug segments.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginName(String);

impl PluginName {
    /// Parses a complete plugin identifier without normalizing its spelling.
    pub fn parse(value: &str) -> Result<Self, PluginNameError> {
        if value.len() > MAX_PLUGIN_NAME_BYTES {
            return Err(PluginNameError::TooLong {
                max_bytes: MAX_PLUGIN_NAME_BYTES,
                actual_bytes: value.len(),
            });
        }

        let segments: Vec<_> = value.split('.').collect();
        if segments.len() > MAX_PLUGIN_NAME_SEGMENTS {
            return Err(PluginNameError::TooManySegments {
                max_segments: MAX_PLUGIN_NAME_SEGMENTS,
                actual_segments: segments.len(),
            });
        }

        for (segment_index, segment) in segments.iter().enumerate() {
            Slug::parse(segment).map_err(|source| PluginNameError::InvalidSlug {
                segment_index,
                source,
            })?;
        }

        Ok(Self(value.to_owned()))
    }

    /// Returns the validated complete plugin identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for PluginName {
    /// Borrows the validated plugin identifier.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PluginName {
    /// Writes the validated plugin identifier.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PluginName {
    type Err = PluginNameError;

    /// Parses a plugin identifier through the invariant-preserving constructor.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Describes why one string cannot be represented as a [`PluginName`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PluginNameError {
    #[error("plugin name exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("plugin name has {actual_segments} segments; at most {max_segments} are allowed")]
    TooManySegments {
        max_segments: usize,
        actual_segments: usize,
    },
    #[error("plugin name segment {segment_index} is invalid: {source}")]
    InvalidSlug {
        segment_index: usize,
        #[source]
        source: SlugError,
    },
}

#[cfg(test)]
mod tests {
    use super::{PluginName, PluginNameError};
    use ora_utils::SlugError;
    use pretty_assertions::assert_eq;

    /// Verifies one- and two-segment plugin identifiers preserve their spelling.
    #[test]
    fn parses_valid_plugin_names() {
        let longest = format!("{}.{}", "a".repeat(63), "b".repeat(63));
        for input in ["weather", "user.ora-weather", longest.as_str()] {
            let Ok(name) = PluginName::parse(input) else {
                panic!("expected {input:?} to be a valid plugin name");
            };
            assert_eq!(name.as_str(), input);
        }
    }

    /// Verifies dot structure and shared slug failures stay distinguishable.
    #[test]
    fn rejects_invalid_plugin_names() {
        assert_eq!(
            PluginName::parse("user.plugin.extra"),
            Err(PluginNameError::TooManySegments {
                max_segments: 2,
                actual_segments: 3,
            })
        );
        assert_eq!(
            PluginName::parse("user."),
            Err(PluginNameError::InvalidSlug {
                segment_index: 1,
                source: SlugError::Empty,
            })
        );
        assert_eq!(
            PluginName::parse("User.weather"),
            Err(PluginNameError::InvalidSlug {
                segment_index: 0,
                source: SlugError::InvalidCharacter {
                    index: 0,
                    character: 'U',
                },
            })
        );

        let over_limit = format!("{}.{}", "a".repeat(64), "b".repeat(64));
        assert_eq!(
            PluginName::parse(&over_limit),
            Err(PluginNameError::TooLong {
                max_bytes: 128,
                actual_bytes: 129,
            })
        );
    }
}

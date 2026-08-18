use std::{fmt, str::FromStr};
use thiserror::Error;

const MAX_SLUG_BYTES: usize = 63;

/// Holds one lowercase ASCII slug segment after syntax and length validation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Slug(String);

impl Slug {
    /// Parses one owned slug segment without normalizing the caller's spelling.
    pub fn parse(value: &str) -> Result<Self, SlugError> {
        if value.is_empty() {
            return Err(SlugError::Empty);
        }
        if value.len() > MAX_SLUG_BYTES {
            return Err(SlugError::TooLong {
                max_bytes: MAX_SLUG_BYTES,
                actual_bytes: value.len(),
            });
        }
        if value.starts_with('-') {
            return Err(SlugError::LeadingHyphen);
        }
        if value.ends_with('-') {
            return Err(SlugError::TrailingHyphen);
        }
        if value.contains("--") {
            return Err(SlugError::ConsecutiveHyphens);
        }

        if let Some((index, character)) = value.char_indices().find(|(_, character)| {
            !character.is_ascii_lowercase() && !character.is_ascii_digit() && *character != '-'
        }) {
            return Err(SlugError::InvalidCharacter { index, character });
        }

        Ok(Self(value.to_owned()))
    }

    /// Returns the validated slug spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Slug {
    /// Borrows the validated slug spelling.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Slug {
    /// Writes the validated slug without normalization.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Slug {
    type Err = SlugError;

    /// Parses a slug through the same invariant-preserving entrypoint as [`Slug::parse`].
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Describes why one string cannot be represented as a [`Slug`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SlugError {
    #[error("slug must not be empty")]
    Empty,
    #[error("slug exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("slug must not start with a hyphen")]
    LeadingHyphen,
    #[error("slug must not end with a hyphen")]
    TrailingHyphen,
    #[error("slug must not contain consecutive hyphens")]
    ConsecutiveHyphens,
    #[error("slug contains invalid character {character:?} at byte {index}")]
    InvalidCharacter { index: usize, character: char },
}

#[cfg(test)]
mod tests {
    use super::{MAX_SLUG_BYTES, Slug, SlugError};
    use pretty_assertions::assert_eq;

    /// Verifies accepted boundary spellings are preserved exactly.
    #[test]
    fn parses_valid_slug_boundaries() {
        let longest = "a".repeat(MAX_SLUG_BYTES);
        let cases = ["a", "ora-weather", "plugin2", longest.as_str()];

        for input in cases {
            let Ok(slug) = Slug::parse(input) else {
                panic!("expected {input:?} to be a valid slug");
            };
            assert_eq!(slug.as_str(), input);
        }
    }

    /// Verifies every syntax category maps to a stable structured error.
    #[test]
    fn rejects_invalid_slug_syntax() {
        let cases = [
            ("", SlugError::Empty),
            ("-ora", SlugError::LeadingHyphen),
            ("ora-", SlugError::TrailingHyphen),
            ("ora--weather", SlugError::ConsecutiveHyphens),
            (
                "Ora",
                SlugError::InvalidCharacter {
                    index: 0,
                    character: 'O',
                },
            ),
            (
                "ora_weather",
                SlugError::InvalidCharacter {
                    index: 3,
                    character: '_',
                },
            ),
            (
                "天气",
                SlugError::InvalidCharacter {
                    index: 0,
                    character: '天',
                },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(Slug::parse(input), Err(expected), "{input}");
        }
    }

    /// Verifies byte length above the supported maximum is rejected before syntax inspection.
    #[test]
    fn rejects_slug_over_length_limit() {
        let input = "a".repeat(MAX_SLUG_BYTES + 1);

        assert_eq!(
            Slug::parse(&input),
            Err(SlugError::TooLong {
                max_bytes: MAX_SLUG_BYTES,
                actual_bytes: MAX_SLUG_BYTES + 1,
            })
        );
    }
}

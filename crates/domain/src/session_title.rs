use thiserror::Error;

/// Limits persisted session titles by Unicode scalar values rather than bytes.
pub const MAX_SESSION_TITLE_CHARS: usize = 255;

/// Reports why an agent-provided session title cannot be persisted.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SessionTitleError {
    #[error("session title must not be blank")]
    Blank,
    #[error("session title exceeds {limit} Unicode scalar values: {actual}")]
    TooLong { limit: usize, actual: usize },
}

/// Represents a normalized, persistable session title.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionTitle(String);

impl SessionTitle {
    /// Validates and normalizes an agent-provided title before it crosses the domain boundary.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, SessionTitleError> {
        let normalized = value.as_ref().trim().to_owned();
        if normalized.is_empty() {
            return Err(SessionTitleError::Blank);
        }

        let actual = normalized.chars().count();
        if actual > MAX_SESSION_TITLE_CHARS {
            return Err(SessionTitleError::TooLong {
                limit: MAX_SESSION_TITLE_CHARS,
                actual,
            });
        }

        Ok(Self(normalized))
    }

    /// Returns the normalized title for persistence and contract mapping.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for SessionTitle {
    /// Serializes the value object as its normalized string representation.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for SessionTitle {
    /// Restores a value object while applying the same validation as fresh input.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{MAX_SESSION_TITLE_CHARS, SessionTitle, SessionTitleError};

    /// Verifies titles are trimmed before they become domain values.
    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            SessionTitle::parse("  New session  ").unwrap().as_str(),
            "New session"
        );
    }

    /// Verifies blank input is rejected instead of becoming an empty persisted title.
    #[test]
    fn rejects_blank_titles() {
        assert_eq!(
            SessionTitle::parse(" \t\n").unwrap_err(),
            SessionTitleError::Blank
        );
    }

    /// Verifies the length invariant counts Unicode scalar values rather than UTF-8 bytes.
    #[test]
    fn counts_unicode_scalar_values() {
        let valid = "界".repeat(MAX_SESSION_TITLE_CHARS);
        let too_long = "界".repeat(MAX_SESSION_TITLE_CHARS + 1);

        assert_eq!(
            SessionTitle::parse(valid).unwrap().as_str().chars().count(),
            MAX_SESSION_TITLE_CHARS
        );
        assert_eq!(
            SessionTitle::parse(too_long).unwrap_err(),
            SessionTitleError::TooLong {
                limit: MAX_SESSION_TITLE_CHARS,
                actual: MAX_SESSION_TITLE_CHARS + 1,
            }
        );
    }
}

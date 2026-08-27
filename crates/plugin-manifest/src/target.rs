use std::{fmt, str::FromStr};
use thiserror::Error;

/// Maximum byte length of a canonical Rust target triple.
const MAX_TARGET_TRIPLE_BYTES: usize = 64;

/// Holds one canonical Rust target triple identifying an installable artifact's OS, architecture,
/// and binary ABI.
///
/// A Hook Target never participates in plugin identity: it only selects which physical artifact a
/// release ships for one host. Matching is exact — two triples must be byte-for-byte equal — so
/// target selection never falls back across architecture, operating system, libc, or ABI.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HookTarget {
    triple: String,
}

impl HookTarget {
    /// Parses a canonical target triple, rejecting empties, control characters, and overlong
    /// values before they enter the domain model.
    pub fn parse(value: &str) -> Result<Self, HookTargetError> {
        if value.is_empty() {
            return Err(HookTargetError::Empty);
        }
        if value.len() > MAX_TARGET_TRIPLE_BYTES {
            return Err(HookTargetError::TooLong {
                max_bytes: MAX_TARGET_TRIPLE_BYTES,
                actual_bytes: value.len(),
            });
        }
        if value.chars().any(char::is_control) {
            return Err(HookTargetError::ContainsControlCharacter);
        }
        // Triples are produced by build scripts and consumed verbatim, so whitespace or any
        // non-ASCII byte is a packaging mistake rather than a portable value.
        if value
            .chars()
            .any(|character| character.is_whitespace() || !character.is_ascii())
        {
            return Err(HookTargetError::NonAscii);
        }

        Ok(Self {
            triple: value.to_owned(),
        })
    }

    /// Returns the canonical triple spelling.
    pub fn as_str(&self) -> &str {
        &self.triple
    }
}

impl fmt::Display for HookTarget {
    /// Writes the canonical triple spelling.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.triple)
    }
}

impl FromStr for HookTarget {
    type Err = HookTargetError;

    /// Parses a target triple through its invariant-preserving constructor.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Reports a target triple that cannot be used as a Hook Target.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HookTargetError {
    #[error("target triple must not be empty")]
    Empty,
    #[error("target triple exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("target triple must not contain control characters")]
    ContainsControlCharacter,
    #[error("target triple must contain ASCII text only")]
    NonAscii,
}

#[cfg(test)]
mod tests {
    use super::{HookTarget, HookTargetError};
    use pretty_assertions::assert_eq;

    /// The first RTK release supports exactly the Windows x86_64 MSVC triple.
    #[test]
    fn parses_the_windows_x86_64_msvc_triple() {
        let target = HookTarget::parse("x86_64-pc-windows-msvc").expect("valid triple");

        assert_eq!(target.as_str(), "x86_64-pc-windows-msvc");
        assert_eq!(target.to_string(), "x86_64-pc-windows-msvc");
    }

    /// Empty, overlong, and non-ASCII triples never enter the domain model.
    #[test]
    fn rejects_invalid_target_triples() {
        assert_eq!(HookTarget::parse(""), Err(HookTargetError::Empty));
        assert_eq!(
            HookTarget::parse(&"a".repeat(65)),
            Err(HookTargetError::TooLong {
                max_bytes: 64,
                actual_bytes: 65,
            })
        );
        assert_eq!(
            HookTarget::parse("x86_64\n"),
            Err(HookTargetError::ContainsControlCharacter)
        );
        assert_eq!(
            HookTarget::parse("x86_64-pc-windows-msvc "),
            Err(HookTargetError::NonAscii)
        );
        assert_eq!(
            HookTarget::parse("x86_64-pc-windows-msvcß"),
            Err(HookTargetError::NonAscii)
        );
    }
}

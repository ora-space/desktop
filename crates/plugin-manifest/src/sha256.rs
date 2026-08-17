use std::{fmt, str::FromStr};
use thiserror::Error;

const SHA256_HEX_LENGTH: usize = 64;

/// Holds a decoded SHA-256 digest so malformed hexadecimal text cannot enter the domain model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Parses exactly 64 hexadecimal bytes into a binary SHA-256 digest.
    pub fn parse(value: &str) -> Result<Self, Sha256DigestError> {
        if value.len() != SHA256_HEX_LENGTH {
            return Err(Sha256DigestError::InvalidLength {
                expected_bytes: SHA256_HEX_LENGTH,
                actual_bytes: value.len(),
            });
        }

        let mut digest = [0_u8; 32];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_value(pair[0]).ok_or(Sha256DigestError::InvalidHexByte {
                index: index * 2,
                byte: pair[0],
            })?;
            let low = hex_value(pair[1]).ok_or(Sha256DigestError::InvalidHexByte {
                index: index * 2 + 1,
                byte: pair[1],
            })?;
            digest[index] = high << 4 | low;
        }

        Ok(Self(digest))
    }

    /// Returns the decoded digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    /// Writes the canonical lowercase hexadecimal digest.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Sha256Digest {
    type Err = Sha256DigestError;

    /// Parses a SHA-256 digest through the invariant-preserving constructor.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Describes malformed SHA-256 hexadecimal text.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum Sha256DigestError {
    #[error("SHA-256 digest must be {expected_bytes} bytes, found {actual_bytes}")]
    InvalidLength {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("SHA-256 digest contains invalid byte 0x{byte:02x} at index {index}")]
    InvalidHexByte { index: usize, byte: u8 },
}

/// Converts one ASCII hexadecimal digit into its numeric value.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Sha256Digest, Sha256DigestError};
    use pretty_assertions::assert_eq;

    /// Verifies uppercase input is accepted and rendered in canonical lowercase form.
    #[test]
    fn canonicalizes_digest_display() {
        let input = "AB".repeat(32);
        let Ok(digest) = Sha256Digest::parse(&input) else {
            panic!("expected valid digest");
        };

        assert_eq!(digest.to_string(), "ab".repeat(32));
        assert_eq!(digest.as_bytes(), &[0xab; 32]);
    }

    /// Verifies digest length and hexadecimal failures remain distinguishable.
    #[test]
    fn rejects_invalid_digest_text() {
        assert_eq!(
            Sha256Digest::parse("00"),
            Err(Sha256DigestError::InvalidLength {
                expected_bytes: 64,
                actual_bytes: 2,
            })
        );

        let input = format!("{}g0", "00".repeat(31));
        assert_eq!(
            Sha256Digest::parse(&input),
            Err(Sha256DigestError::InvalidHexByte {
                index: 62,
                byte: b'g',
            })
        );
    }
}

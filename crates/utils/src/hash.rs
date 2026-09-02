//! Streaming SHA-256 digest helpers that never load large inputs fully into memory.

use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

/// Read buffer size used while hashing a reader; 64 KiB balances syscalls and CPU pressure.
const BUFFER_SIZE: usize = 64 * 1024;

/// Streams `reader` through SHA-256 and returns the lowercase hex digest.
///
/// The reader is consumed incrementally so callers can digest arbitrarily large downloads
/// without buffering them, matching the needs of `.orax` release verification.
pub fn sha256_reader(reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; BUFFER_SIZE];
    let mut reader = reader;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(digest_hex(&hasher.finalize()))
}

/// Streams the file at `path` through SHA-256, returning the lowercase hex digest.
///
/// Wraps [`sha256_reader`] so callers that already have an open handle can reuse either API.
pub fn sha256_file(path: impl AsRef<Path>) -> io::Result<String> {
    sha256_reader(std::fs::File::open(path)?)
}

/// Returns the lowercase hex SHA-256 digest of an in-memory byte slice.
///
/// Kept separate from the streaming helpers because callers that digest a short, already-resident
/// value (an identifier, a canonical URL) should not have to wrap it in a reader.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    digest_hex(&hasher.finalize())
}

/// Renders a digest as lowercase hex without aggregating hex formatting per call.
fn digest_hex(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX_DIGITS[(byte >> 4) as usize]));
        output.push(char::from(HEX_DIGITS[(byte & 0x0f) as usize]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{sha256_file, sha256_hex, sha256_reader};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::io::Cursor;
    use tempfile::TempDir;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// Verifies the digest of an empty stream against the published SHA-256 vector.
    #[test]
    fn hashes_empty_stream() {
        assert_eq!(
            sha256_reader(Cursor::new(Vec::<u8>::new())).unwrap(),
            EMPTY_SHA256
        );
    }

    /// Verifies the digest of short known input against the published SHA-256 vector.
    #[test]
    fn hashes_known_input() {
        assert_eq!(
            sha256_reader(Cursor::new(b"abc".to_vec())).unwrap(),
            ABC_SHA256
        );
    }

    /// Confirms streaming a large body matches reading the same bytes from disk.
    #[test]
    fn file_and_reader_agree_across_many_blocks() {
        let temp_dir = TempDir::new().unwrap();
        let payload: Vec<u8> = (0..=u8::MAX).cycle().take(2 * 1024 * 1024).collect();
        let path = temp_dir.path().join("payload.bin");
        fs::write(&path, &payload).unwrap();

        let from_disk = sha256_file(&path).unwrap();
        let from_memory = sha256_reader(Cursor::new(payload)).unwrap();
        assert_eq!(from_disk, from_memory);
    }

    /// Verifies in-memory digests agree with the streaming path on the same bytes.
    #[test]
    fn in_memory_and_streaming_digests_agree() {
        assert_eq!(
            (sha256_hex(b""), sha256_hex(b"abc")),
            (EMPTY_SHA256.to_string(), ABC_SHA256.to_string()),
        );
    }

    /// Verifies the digest is always lowercase hex.
    #[test]
    fn digest_is_lowercase_hex() {
        let digest = sha256_reader(Cursor::new(b"abc".to_vec())).unwrap();
        assert_eq!(digest, digest.to_ascii_lowercase());
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()) && digest.len() == 64);
    }
}

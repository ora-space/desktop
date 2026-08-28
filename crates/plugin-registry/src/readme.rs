//! Bounded reads of the README every marketplace listing ships beside `orax.toml`.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use thiserror::Error;

/// The size cap for a marketplace README, so an arbitrarily large document is never pulled into
/// memory when the marketplace detail page renders it.
pub const MAX_README_BYTES: u64 = 256 * 1024;

/// Reports why one marketplace README could not be turned into text for the detail page.
#[derive(Debug, Error)]
pub enum ReadmeReadError {
    /// Wraps registry-tree discovery failures while locating the listing's manifest.
    #[error("marketplace README resolution failed: {0}")]
    Registry(#[from] crate::RegistryError),
    #[error("failed to read the marketplace README: {0}")]
    Unreadable(#[from] io::Error),
    #[error("the marketplace README is not valid UTF-8 text")]
    NotUtf8,
    #[error("the marketplace README exceeds the {max} byte read limit")]
    TooLarge { max: u64 },
}

/// Reads the bounded README that sits beside one registry manifest.
///
/// A missing file is the ordinary no-documentation case and reads as `None`, so a listing that
/// ships no README still opens its detail page; any other read or validation problem is reported
/// so the UI can distinguish an absent document from an unusable one.
pub(crate) fn read_beside_manifest(
    manifest_path: &Path,
) -> Result<Option<String>, ReadmeReadError> {
    let Some(directory) = manifest_path.parent() else {
        return Ok(None);
    };
    match read_bounded_utf8(&directory.join("README.md")) {
        Ok(content) => Ok(Some(content)),
        Err(ReadmeReadError::Unreadable(error)) if error.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Reads one file bounded to one byte past [`MAX_README_BYTES`] so an oversized document reports
/// `TooLarge` instead of being silently truncated or read into memory in full.
fn read_bounded_utf8(path: &Path) -> Result<String, ReadmeReadError> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_README_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_README_BYTES {
        return Err(ReadmeReadError::TooLarge {
            max: MAX_README_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|_| ReadmeReadError::NotUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Returns the manifest path of a listing whose README is written by the caller.
    fn listing_with_readme(readme: &str) -> (TempDir, std::path::PathBuf) {
        let root = TempDir::new().expect("temp dir");
        let dir = root
            .path()
            .join("registry")
            .join("o")
            .join("ora-space.weather");
        fs::create_dir_all(&dir).expect("create listing dir");
        fs::write(dir.join("orax.toml"), "resolver = 1\n").expect("write manifest");
        fs::write(dir.join("README.md"), readme).expect("write readme");
        (root, dir.join("orax.toml"))
    }

    /// Verifies the README beside a manifest is returned as UTF-8 text.
    #[test]
    fn reads_the_readme_beside_a_manifest() {
        let (_root, manifest) = listing_with_readme("# Weather\n\nLive forecasts.");

        assert_eq!(
            read_beside_manifest(&manifest).expect("read readme"),
            Some("# Weather\n\nLive forecasts.".to_string())
        );
    }

    /// Verifies a listing without a README is the ordinary no-documentation case.
    #[test]
    fn missing_readme_reads_as_none() {
        let root = TempDir::new().expect("temp dir");
        let dir = root
            .path()
            .join("registry")
            .join("o")
            .join("ora-space.weather");
        fs::create_dir_all(&dir).expect("create listing dir");
        fs::write(dir.join("orax.toml"), "resolver = 1\n").expect("write manifest");

        assert_eq!(
            read_beside_manifest(&dir.join("orax.toml")).expect("read readme"),
            None
        );
    }

    /// Verifies a README that is not UTF-8 is reported instead of rendered as garbage.
    #[test]
    fn non_utf8_readme_is_reported() {
        let (_root, manifest) = listing_with_readme("é");
        fs::write(
            manifest.parent().expect("listing dir").join("README.md"),
            [0xFF, 0xFE, 0x00, 0x41],
        )
        .expect("write binary readme");

        assert!(matches!(
            read_beside_manifest(&manifest).expect_err("non-utf8 readme"),
            ReadmeReadError::NotUtf8
        ));
    }

    /// Verifies an oversized README reports the limit instead of being truncated silently.
    #[test]
    fn oversized_readme_is_reported() {
        let (_root, manifest) = listing_with_readme("");
        fs::write(
            manifest.parent().expect("listing dir").join("README.md"),
            "a".repeat(MAX_README_BYTES as usize + 1),
        )
        .expect("write oversized readme");

        assert!(matches!(
            read_beside_manifest(&manifest).expect_err("oversized readme"),
            ReadmeReadError::TooLarge {
                max: MAX_README_BYTES
            }
        ));
    }
}

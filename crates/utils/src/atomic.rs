//! Atomic replacement of a single file by writing through a same-directory temporary file.

use std::io::{self, Write};
use std::path::Path;
use tempfile::NamedTempFile;

/// Atomically replaces `path` with `contents`.
///
/// The payload is flushed and synced into a temporary file in the target's directory, then
/// renamed over the destination. Readers never observe a partially written file, and a failed
/// write leaves any existing destination untouched. The target's parent directory must exist.
pub fn write(path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
    write_with_prepare(path, contents, |_| Ok(()))
}

/// Atomically replaces `path` after preparing the same-directory temporary file.
///
/// The preparation hook runs after content sync but before the final rename, which lets callers
/// apply permissions or other file metadata without changing the destination on failure.
pub fn write_with_prepare(
    path: impl AsRef<Path>,
    contents: &[u8],
    prepare: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let target = path.as_ref();
    let parent = target
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty());
    let mut temporary = match parent {
        Some(parent) => NamedTempFile::new_in(parent)?,
        None => NamedTempFile::new()?,
    };
    temporary.write_all(contents)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    prepare(temporary.path())?;
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Writes fresh content to a path that did not previously exist.
    #[test]
    fn writes_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("index.json");
        write(&target, b"first").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"first");
    }

    /// Replaces existing content so the destination is never left partially updated.
    #[test]
    fn overwrites_existing_file() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("index.json");
        fs::write(&target, b"old").unwrap();

        write(&target, b"new").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    /// Propagates the missing-directory error instead of silently falling back to cwd writes.
    #[test]
    fn refuses_missing_parent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("missing").join("index.json");

        assert!(write(&target, b"data").is_err());
    }

    /// A preparation failure leaves the previous destination content untouched.
    #[test]
    fn prepare_failure_does_not_replace_the_destination() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("index.json");
        fs::write(&target, b"old").unwrap();

        let error = super::write_with_prepare(&target, b"new", |_| {
            Err(std::io::Error::other("injected preparation failure"))
        })
        .expect_err("preparation must fail");

        assert_eq!(
            (error.to_string(), fs::read(&target).unwrap()),
            ("injected preparation failure".to_string(), b"old".to_vec())
        );
    }
}

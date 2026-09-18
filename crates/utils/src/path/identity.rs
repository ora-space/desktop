use serde::{Deserialize, Serialize};
use std::{fs, io, os::unix::fs::MetadataExt, path::Path, time::UNIX_EPOCH};

/// Stable Unix directory identity, including birth time to detect ordinary inode reuse.
/// Filesystems without birth-time support are rejected rather than given weaker restart evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DirectoryIdentity {
    device: u64,
    inode: u64,
    created_seconds: u64,
    created_nanos: u32,
}

impl DirectoryIdentity {
    /// Reads a real directory without accepting a final symlink or treating path spelling as identity.
    pub fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other("expected a directory, not a link"));
        }
        let created = metadata
            .created()?
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            created_seconds: created.as_secs(),
            created_nanos: created.subsec_nanos(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Content changes preserve identity, while directory replacement and symlinks do not.
    #[test]
    fn directory_identity_survives_content_changes_but_rejects_replacement() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("directory");
        fs::create_dir(&path)?;
        let identity = DirectoryIdentity::read(&path)?;
        fs::write(path.join("content"), "data")?;
        assert_eq!(DirectoryIdentity::read(&path)?, identity);
        let old = root.path().join("old");
        fs::rename(&path, &old)?;
        fs::create_dir(&path)?;
        assert_ne!(DirectoryIdentity::read(&path)?, identity);
        let link = root.path().join("link");
        std::os::unix::fs::symlink(old, &link)?;
        assert!(DirectoryIdentity::read(&link).is_err());
        Ok(())
    }
}

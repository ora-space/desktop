//! Safe, deterministic operations over ordinary directory trees.
//!
//! Traversal never follows links and rejects special filesystem entries. Fingerprints intentionally
//! exclude timestamps, ownership, ACLs, and other host metadata that cannot be reproduced across
//! supported platforms.

use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A stable SHA-256 identity for the portable contents of one directory tree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DirectoryFingerprint(String);

impl DirectoryFingerprint {
    /// Reconstructs a previously persisted fingerprint after validating its representation.
    pub fn parse(value: impl Into<String>) -> Result<Self, DirectoryTreeError> {
        let value = value.into();
        let hex = value
            .strip_prefix("sha256:")
            .ok_or_else(|| DirectoryTreeError::InvalidFingerprint(value.clone()))?;
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DirectoryTreeError::InvalidFingerprint(value));
        }
        Ok(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
    }

    /// Returns the persistence-safe algorithm-prefixed representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DirectoryFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reports a directory-tree safety or I/O failure without hiding the affected path.
#[derive(Debug, Error)]
pub enum DirectoryTreeError {
    #[error("directory tree root is unavailable: {path:?}")]
    RootUnavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("directory tree contains a symbolic link: {path:?}")]
    LinkRejected { path: PathBuf },
    #[error("directory tree contains a special entry: {path:?}")]
    SpecialEntryRejected { path: PathBuf },
    #[error("directory tree contains a non-UTF-8 or non-portable path: {path:?}")]
    NonPortablePath { path: PathBuf },
    #[error("directory destination is not empty: {path:?}")]
    DestinationNotEmpty { path: PathBuf },
    #[error("invalid directory fingerprint: {0}")]
    InvalidFingerprint(String),
    #[error("directory tree I/O failed: {path:?}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Computes a deterministic fingerprint while omitting caller-owned root metadata files.
pub fn fingerprint_directory(
    root: &Path,
    ignored_root_files: &[&OsStr],
) -> Result<DirectoryFingerprint, DirectoryTreeError> {
    if !root.is_dir() {
        return Err(DirectoryTreeError::RootUnavailable {
            path: root.to_path_buf(),
            source: io::Error::new(io::ErrorKind::NotFound, "directory is unavailable"),
        });
    }

    let ignored = ignored_root_files
        .iter()
        .map(|name| (*name).to_os_string())
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    collect_entries(root, root, &ignored, &mut entries)?;
    entries.sort_by(|left, right| left.relative.cmp(&right.relative));

    let mut digest = Sha256::new();
    digest.update(b"ora-directory-fingerprint-v1\0");
    for entry in entries {
        digest.update(entry.kind.tag());
        digest.update((entry.relative.len() as u64).to_le_bytes());
        digest.update(entry.relative.as_bytes());
        digest.update([entry.executable as u8]);
        if entry.kind == EntryKind::File {
            let path = root.join(Path::new(&entry.relative));
            let mut file = fs::File::open(&path).map_err(|source| DirectoryTreeError::Io {
                path: path.clone(),
                source,
            })?;
            let size = file
                .metadata()
                .map_err(|source| DirectoryTreeError::Io {
                    path: path.clone(),
                    source,
                })?
                .len();
            digest.update(size.to_le_bytes());
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|source| DirectoryTreeError::Io {
                        path: path.clone(),
                        source,
                    })?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
        }
    }

    Ok(DirectoryFingerprint(format!(
        "sha256:{:x}",
        digest.finalize()
    )))
}

/// Copies an ordinary tree without following links and preserves portable executable intent.
pub fn copy_directory(
    source: &Path,
    destination: &Path,
    forbidden_root_files: &[&OsStr],
) -> Result<(), DirectoryTreeError> {
    if !source.is_dir() {
        return Err(DirectoryTreeError::RootUnavailable {
            path: source.to_path_buf(),
            source: io::Error::new(io::ErrorKind::NotFound, "directory is unavailable"),
        });
    }
    if destination.exists()
        && destination
            .read_dir()
            .map_err(|source| DirectoryTreeError::Io {
                path: destination.to_path_buf(),
                source,
            })?
            .next()
            .is_some()
    {
        return Err(DirectoryTreeError::DestinationNotEmpty {
            path: destination.to_path_buf(),
        });
    }

    fs::create_dir_all(destination).map_err(|source| DirectoryTreeError::Io {
        path: destination.to_path_buf(),
        source,
    })?;
    let forbidden = forbidden_root_files
        .iter()
        .map(|name| (*name).to_os_string())
        .collect::<BTreeSet<_>>();
    copy_entries(source, source, destination, &forbidden)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryKind {
    Directory,
    File,
}

impl EntryKind {
    fn tag(self) -> &'static [u8] {
        match self {
            Self::Directory => b"d",
            Self::File => b"f",
        }
    }
}

#[derive(Debug)]
struct TreeEntry {
    relative: String,
    kind: EntryKind,
    executable: bool,
}

/// Collects safe entries before hashing so host enumeration order cannot affect identity.
fn collect_entries(
    root: &Path,
    directory: &Path,
    ignored_root_files: &BTreeSet<OsString>,
    entries: &mut Vec<TreeEntry>,
) -> Result<(), DirectoryTreeError> {
    let children = fs::read_dir(directory).map_err(|source| DirectoryTreeError::Io {
        path: directory.to_path_buf(),
        source,
    })?;
    for child in children {
        let child = child.map_err(|source| DirectoryTreeError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = child.path();
        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| DirectoryTreeError::NonPortablePath { path: path.clone() })?;
        if relative_path.components().count() == 1
            && ignored_root_files.contains(&child.file_name())
        {
            continue;
        }
        let relative = portable_relative(relative_path, &path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|source| DirectoryTreeError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(DirectoryTreeError::LinkRejected { path });
        }
        if metadata.is_dir() {
            entries.push(TreeEntry {
                relative,
                kind: EntryKind::Directory,
                executable: false,
            });
            collect_entries(root, &path, ignored_root_files, entries)?;
        } else if metadata.is_file() {
            entries.push(TreeEntry {
                relative,
                kind: EntryKind::File,
                executable: is_executable(&metadata),
            });
        } else {
            return Err(DirectoryTreeError::SpecialEntryRejected { path });
        }
    }
    Ok(())
}

/// Recursively copies after validating each source entry immediately before opening it.
fn copy_entries(
    root: &Path,
    directory: &Path,
    destination: &Path,
    forbidden_root_files: &BTreeSet<OsString>,
) -> Result<(), DirectoryTreeError> {
    for child in fs::read_dir(directory).map_err(|source| DirectoryTreeError::Io {
        path: directory.to_path_buf(),
        source,
    })? {
        let child = child.map_err(|source| DirectoryTreeError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        let source_path = child.path();
        let relative =
            source_path
                .strip_prefix(root)
                .map_err(|_| DirectoryTreeError::NonPortablePath {
                    path: source_path.clone(),
                })?;
        if relative.components().count() == 1 && forbidden_root_files.contains(&child.file_name()) {
            return Err(DirectoryTreeError::SpecialEntryRejected { path: source_path });
        }
        portable_relative(relative, &source_path)?;
        let metadata =
            fs::symlink_metadata(&source_path).map_err(|source| DirectoryTreeError::Io {
                path: source_path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(DirectoryTreeError::LinkRejected { path: source_path });
        }
        let destination_path = destination.join(relative);
        if metadata.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|source| DirectoryTreeError::Io {
                path: destination_path.clone(),
                source,
            })?;
            copy_entries(root, &source_path, destination, forbidden_root_files)?;
        } else if metadata.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).map_err(|source| DirectoryTreeError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            let mut input =
                fs::File::open(&source_path).map_err(|source| DirectoryTreeError::Io {
                    path: source_path.clone(),
                    source,
                })?;
            let mut output =
                fs::File::create(&destination_path).map_err(|source| DirectoryTreeError::Io {
                    path: destination_path.clone(),
                    source,
                })?;
            io::copy(&mut input, &mut output).map_err(|source| DirectoryTreeError::Io {
                path: destination_path.clone(),
                source,
            })?;
            output.flush().map_err(|source| DirectoryTreeError::Io {
                path: destination_path.clone(),
                source,
            })?;
            set_executable(&destination_path, is_executable(&metadata))?;
        } else {
            return Err(DirectoryTreeError::SpecialEntryRejected { path: source_path });
        }
    }
    Ok(())
}

/// Encodes host relative paths with a slash separator after rejecting ambiguous components.
fn portable_relative(relative: &Path, full_path: &Path) -> Result<String, DirectoryTreeError> {
    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(segment) = component else {
            return Err(DirectoryTreeError::NonPortablePath {
                path: full_path.to_path_buf(),
            });
        };
        let segment = segment
            .to_str()
            .ok_or_else(|| DirectoryTreeError::NonPortablePath {
                path: full_path.to_path_buf(),
            })?;
        if segment.contains(['/', '\\']) {
            return Err(DirectoryTreeError::NonPortablePath {
                path: full_path.to_path_buf(),
            });
        }
        parts.push(segment);
    }
    Ok(parts.join("/"))
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), DirectoryTreeError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path).map_err(|source| DirectoryTreeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut permissions = metadata.permissions();
    let current = permissions.mode();
    let mode = if executable {
        current | 0o111
    } else {
        current & !0o111
    };
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions).map_err(|source| DirectoryTreeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), DirectoryTreeError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{copy_directory, fingerprint_directory};
    use pretty_assertions::assert_eq;
    use std::ffi::OsStr;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn fingerprints_bytes_and_ignores_selected_root_metadata() {
        let root = TempDir::new().expect("create fixture");
        fs::create_dir(root.path().join("scripts")).expect("create scripts");
        fs::write(root.path().join("SKILL.md"), b"manifest").expect("write manifest");
        fs::write(root.path().join("scripts").join("run"), [0, 1, 2]).expect("write binary");
        fs::write(root.path().join(".marker"), b"one").expect("write marker");
        let first =
            fingerprint_directory(root.path(), &[OsStr::new(".marker")]).expect("fingerprint tree");
        fs::write(root.path().join(".marker"), b"two").expect("update marker");
        let second =
            fingerprint_directory(root.path(), &[OsStr::new(".marker")]).expect("fingerprint tree");
        assert_eq!(first, second);
    }

    #[test]
    fn copies_binary_tree_without_copying_forbidden_metadata() {
        let source = TempDir::new().expect("create source");
        let destination = TempDir::new().expect("create destination parent");
        let target = destination.path().join("target");
        fs::write(source.path().join("SKILL.md"), [0, 255, 1]).expect("write binary");

        copy_directory(source.path(), &target, &[OsStr::new(".ora-managed.json")])
            .expect("copy tree");

        assert_eq!(
            fs::read(target.join("SKILL.md")).expect("read copy"),
            [0, 255, 1]
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        use std::os::unix::fs::symlink;
        let source = TempDir::new().expect("create source");
        symlink("missing", source.path().join("linked")).expect("create link");
        assert!(fingerprint_directory(source.path(), &[]).is_err());
    }
}

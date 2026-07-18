use crate::{PluginLimits, SafeDeleteError, audit_no_named_streams};
use ora_plugin_protocol::ContentDigest;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path};

const TREE_DIGEST_DOMAIN: &[u8] = b"ora-plugin-tree-v1\0";
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Fresh proof of one no-follow tree enumeration and its unambiguous v1 digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDigestProof {
    pub digest: ContentDigest,
    pub file_count: u64,
    pub total_bytes: u64,
    pub files: Vec<DigestFileProof>,
}

/// One sorted regular-file input to the aggregate tree digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestFileProof {
    pub relative_path: String,
    pub file_bytes: u64,
    pub sha256: [u8; 32],
}

/// Filesystem failures that invalidate package identity or safety proof.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackageFsError {
    #[error("package root is not a regular directory")]
    InvalidRoot,
    #[error("package contains symlink, reparse, or non-regular object: {relative_path}")]
    UnsupportedObject { relative_path: String },
    #[error("package contains reserved Host metadata directory .ora")]
    ReservedOraDirectory,
    #[error("package path cannot be represented as stable UTF-8: {relative_path}")]
    UnrepresentablePath { relative_path: String },
    #[error("package contains a Windows case-insensitive path collision")]
    CaseCollision,
    #[error("package exceeds {budget} budget")]
    BudgetExceeded { budget: &'static str },
    #[error("package file has more than one hard link: {relative_path}")]
    HardLink { relative_path: String },
    #[error("package object contains a named stream: {relative_path}")]
    NamedStream { relative_path: String },
    #[error("package filesystem operation failed: {message}")]
    Io { message: String },
}

/// Controls whether Host-owned `.ora` metadata participates in traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageTreeMode {
    Candidate,
    InstalledContent,
}

/// Computes the design-specified tree digest over sorted regular package files.
pub fn compute_tree_digest(
    root: &Path,
    limits: &PluginLimits,
    mode: PackageTreeMode,
) -> Result<TreeDigestProof, PackageFsError> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(io_error)?;
    if !root_metadata.is_dir()
        || root_metadata.file_type().is_symlink()
        || is_reparse(&root_metadata)
    {
        return Err(PackageFsError::InvalidRoot);
    }
    audit_no_named_streams(root).map_err(|error| audit_error(error, String::new()))?;

    let mut stack = vec![(root.to_path_buf(), 0usize, true)];
    let mut files = BTreeMap::new();
    let mut case_keys = BTreeSet::new();
    let mut total_bytes = 0u64;

    while let Some((directory, depth, include_in_digest)) = stack.pop() {
        if depth > limits.maximum_directory_depth {
            return Err(PackageFsError::BudgetExceeded {
                budget: "directoryDepth",
            });
        }
        let entries = std::fs::read_dir(&directory).map_err(io_error)?;
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| PackageFsError::Io {
                    message: error.to_string(),
                })?;
            let relative_path = normalize_relative_path(relative)?;
            let is_host_metadata = depth == 0 && relative_path.eq_ignore_ascii_case(".ora");
            if is_host_metadata {
                match mode {
                    PackageTreeMode::Candidate => {
                        return Err(PackageFsError::ReservedOraDirectory);
                    }
                    PackageTreeMode::InstalledContent => {}
                }
            }

            let case_key = relative_path.to_lowercase();
            if !case_keys.insert(case_key) {
                return Err(PackageFsError::CaseCollision);
            }

            let metadata = std::fs::symlink_metadata(&path).map_err(io_error)?;
            if metadata.file_type().is_symlink() || is_reparse(&metadata) {
                return Err(PackageFsError::UnsupportedObject { relative_path });
            }
            audit_no_named_streams(&path)
                .map_err(|error| audit_error(error, relative_path.clone()))?;
            if metadata.is_dir() {
                stack.push((
                    path,
                    depth.saturating_add(1),
                    include_in_digest && !is_host_metadata,
                ));
                continue;
            }
            if !metadata.is_file() {
                return Err(PackageFsError::UnsupportedObject { relative_path });
            }
            if hard_link_count(&path)? != 1 {
                return Err(PackageFsError::HardLink { relative_path });
            }
            if !include_in_digest || is_host_metadata {
                continue;
            }
            let file_bytes = metadata.len();
            if file_bytes > limits.maximum_file_bytes {
                return Err(PackageFsError::BudgetExceeded {
                    budget: "singleFileBytes",
                });
            }
            total_bytes =
                total_bytes
                    .checked_add(file_bytes)
                    .ok_or(PackageFsError::BudgetExceeded {
                        budget: "totalBytes",
                    })?;
            if total_bytes > limits.maximum_total_bytes {
                return Err(PackageFsError::BudgetExceeded {
                    budget: "totalBytes",
                });
            }
            let file_count = u64::try_from(files.len()).unwrap_or(u64::MAX) + 1;
            if file_count > limits.maximum_file_count {
                return Err(PackageFsError::BudgetExceeded {
                    budget: "fileCount",
                });
            }
            let sha256 = hash_file(&path)?;
            audit_no_named_streams(&path)
                .map_err(|error| audit_error(error, relative_path.clone()))?;
            files.insert(
                relative_path.clone(),
                DigestFileProof {
                    relative_path,
                    file_bytes,
                    sha256,
                },
            );
        }
    }

    let mut aggregate = Sha256::new();
    aggregate.update(TREE_DIGEST_DOMAIN);
    for proof in files.values() {
        let path_bytes = proof.relative_path.as_bytes();
        let path_length =
            u32::try_from(path_bytes.len()).map_err(|_| PackageFsError::BudgetExceeded {
                budget: "relativePathBytes",
            })?;
        aggregate.update(path_length.to_be_bytes());
        aggregate.update(path_bytes);
        aggregate.update(proof.file_bytes.to_be_bytes());
        aggregate.update(proof.sha256);
    }
    let digest_bytes: [u8; 32] = aggregate.finalize().into();
    let digest =
        ContentDigest::parse(format!("sha256:{}", encode_hex(&digest_bytes))).map_err(|error| {
            PackageFsError::Io {
                message: error.to_string(),
            }
        })?;
    Ok(TreeDigestProof {
        digest,
        file_count: files.len() as u64,
        total_bytes,
        files: files.into_values().collect(),
    })
}

/// Maps the shared Windows stream audit into package-specific stable diagnostics.
fn audit_error(error: SafeDeleteError, relative_path: String) -> PackageFsError {
    match error {
        SafeDeleteError::NamedStream => PackageFsError::NamedStream { relative_path },
        SafeDeleteError::UnsupportedObject => PackageFsError::UnsupportedObject { relative_path },
        SafeDeleteError::MultipleHardLinks => PackageFsError::HardLink { relative_path },
        SafeDeleteError::OutsideAllowedRoot
        | SafeDeleteError::RefusedAllowedRoot
        | SafeDeleteError::InvalidRelativePath
        | SafeDeleteError::IdentityChanged
        | SafeDeleteError::Io => PackageFsError::Io {
            message: "filesystem object audit failed".to_owned(),
        },
    }
}

/// Hashes a file through a bounded buffer instead of loading package bytes into memory.
fn hash_file(path: &Path) -> Result<[u8; 32], PackageFsError> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// Converts platform components into the digest's required UTF-8 `/` form.
fn normalize_relative_path(path: &Path) -> Result<String, PackageFsError> {
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let segment =
                    segment
                        .to_str()
                        .ok_or_else(|| PackageFsError::UnrepresentablePath {
                            relative_path: path.to_string_lossy().into_owned(),
                        })?;
                if segment.is_empty() || segment == "." || segment == ".." {
                    return Err(PackageFsError::UnrepresentablePath {
                        relative_path: path.to_string_lossy().into_owned(),
                    });
                }
                segments.push(segment);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PackageFsError::UnrepresentablePath {
                    relative_path: path.to_string_lossy().into_owned(),
                });
            }
        }
    }
    Ok(segments.join("/"))
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn hard_link_count(path: &Path) -> Result<u32, PackageFsError> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let file = File::open(path).map_err(io_error)?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(information.nNumberOfLinks)
}

#[cfg(unix)]
fn hard_link_count(path: &Path) -> Result<u32, PackageFsError> {
    use std::os::unix::fs::MetadataExt;

    let links = std::fs::symlink_metadata(path).map_err(io_error)?.nlink();
    u32::try_from(links).map_err(|_| PackageFsError::HardLink {
        relative_path: path.to_string_lossy().into_owned(),
    })
}

/// Encodes one digest as canonical lowercase hex without an extra dependency.
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Removes platform-specific I/O error types from stable package validation errors.
fn io_error(error: io::Error) -> PackageFsError {
    PackageFsError::Io {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PackageFsError, PackageTreeMode, compute_tree_digest};
    use crate::PluginLimits;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Proves digest stability is independent of file creation order.
    #[test]
    fn computes_stable_sorted_tree_digest() {
        let first =
            TempDir::new().unwrap_or_else(|error| panic!("expected first temp directory: {error}"));
        let second = TempDir::new()
            .unwrap_or_else(|error| panic!("expected second temp directory: {error}"));
        fs::write(first.path().join("b.txt"), b"b")
            .unwrap_or_else(|error| panic!("expected first b write: {error}"));
        fs::write(first.path().join("a.txt"), b"a")
            .unwrap_or_else(|error| panic!("expected first a write: {error}"));
        fs::write(second.path().join("a.txt"), b"a")
            .unwrap_or_else(|error| panic!("expected second a write: {error}"));
        fs::write(second.path().join("b.txt"), b"b")
            .unwrap_or_else(|error| panic!("expected second b write: {error}"));

        let limits = PluginLimits::default();
        let first_proof = compute_tree_digest(first.path(), &limits, PackageTreeMode::Candidate)
            .unwrap_or_else(|error| panic!("expected first digest: {error}"));
        let second_proof = compute_tree_digest(second.path(), &limits, PackageTreeMode::Candidate)
            .unwrap_or_else(|error| panic!("expected second digest: {error}"));
        assert_eq!(first_proof, second_proof);
    }

    /// Candidate packages cannot pre-create Host-owned receipt metadata.
    #[test]
    fn rejects_candidate_ora_directory() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected temp directory: {error}"));
        fs::create_dir(root.path().join(".ora"))
            .unwrap_or_else(|error| panic!("expected .ora directory: {error}"));
        assert_eq!(
            compute_tree_digest(
                root.path(),
                &PluginLimits::default(),
                PackageTreeMode::Candidate
            ),
            Err(PackageFsError::ReservedOraDirectory)
        );
    }
}

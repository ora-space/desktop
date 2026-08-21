//! Serves `ora/storage/*` for one plugin process, confined to that plugin's data directory.
//!
//! The plugin never names itself: a `PluginStorage` is built at launch for exactly one plugin
//! and resolves every logical path below `<data-dir>/plugins/data/<namespace>/<name>/`. Logical
//! paths are portable relative paths; anything absolute, parent-traversing, symlinked, or under
//! the host-owned `web-profile/` directory is refused before the filesystem is touched.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ora_plugin_runtime::{HostRequestError, HostRequestHandler};
use ora_utils::path::{
    CanonicalPathRoot, PathContainmentError, PortableRelativePath,
    canonicalize_longest_existing_prefix,
};
use serde_json::{Value, json};
use std::fs;
use std::io;
use std::path::PathBuf;

/// Lists the entries directly below a logical directory.
pub const STORAGE_LIST_METHOD: &str = "ora/storage/list";
/// Reads one whole file as base64.
pub const STORAGE_READ_METHOD: &str = "ora/storage/read";
/// Replaces one whole file from base64, creating parent directories.
pub const STORAGE_WRITE_METHOD: &str = "ora/storage/write";
/// Removes one file or directory tree.
pub const STORAGE_REMOVE_METHOD: &str = "ora/storage/remove";

/// Largest file `read` returns and `write` accepts.
///
/// The protocol frame is capped at 16 MiB and base64 inflates content by one third, so 8 MiB of
/// bytes (about 10.7 MiB encoded, plus envelope) is the largest payload that fits reliably.
pub const MAX_STORAGE_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Host-owned webview site data below the data directory; never part of the logical namespace.
const WEB_PROFILE_DIRECTORY: &str = "web-profile";

/// JSON-RPC codes, one per error kind, so a plugin can branch on either `code` or `data.kind`.
const INVALID_PARAMS_CODE: i64 = -32602;
const NOT_FOUND_CODE: i64 = -32004;
const TOO_LARGE_CODE: i64 = -32005;
const IO_CODE: i64 = -32000;

/// Stable classification of a storage failure, serialized as `data.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageErrorKind {
    /// The request params are malformed (missing `path`, undecodable base64).
    InvalidParams,
    /// The logical path is outside the plugin's namespace or not a portable relative path.
    InvalidPath,
    NotFound,
    TooLarge,
    Io,
}

impl StorageErrorKind {
    /// The wire spelling of the kind.
    fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_params",
            Self::InvalidPath => "invalid_path",
            Self::NotFound => "not_found",
            Self::TooLarge => "too_large",
            Self::Io => "io",
        }
    }

    /// The JSON-RPC code paired with the kind.
    fn code(self) -> i64 {
        match self {
            Self::InvalidParams | Self::InvalidPath => INVALID_PARAMS_CODE,
            Self::NotFound => NOT_FOUND_CODE,
            Self::TooLarge => TOO_LARGE_CODE,
            Self::Io => IO_CODE,
        }
    }
}

/// One storage failure before it is rendered as a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    pub kind: StorageErrorKind,
    pub message: String,
}

impl StorageError {
    fn new(kind: StorageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Maps a containment failure from `ora-utils` onto the storage error space.
    fn from_containment(error: PathContainmentError) -> Self {
        match error {
            PathContainmentError::PathNotFound { .. } => {
                Self::new(StorageErrorKind::NotFound, "path does not exist")
            }
            PathContainmentError::OutsideRoot { .. }
            | PathContainmentError::NonPortablePath { .. }
            | PathContainmentError::NonCanonicalPath { .. }
            | PathContainmentError::NonUtf8Path { .. }
            | PathContainmentError::PathNotAbsolute { .. } => Self::new(
                StorageErrorKind::InvalidPath,
                "path escapes the plugin data directory",
            ),
            PathContainmentError::RootUnavailable { .. } | PathContainmentError::Io { .. } => {
                Self::new(StorageErrorKind::Io, error.to_string())
            }
        }
    }

    fn from_io(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::new(StorageErrorKind::NotFound, "path does not exist")
        } else {
            Self::new(StorageErrorKind::Io, error.to_string())
        }
    }
}

impl From<StorageError> for HostRequestError {
    fn from(error: StorageError) -> Self {
        HostRequestError::new(error.kind.code(), error.message)
            .with_data(json!({ "kind": error.kind.as_str() }))
    }
}

/// One entry of a `list` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEntry {
    pub name: String,
    pub kind: StorageEntryKind,
    pub size_bytes: u64,
}

/// Only regular files and directories are exposed; symlinks and special files are hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEntryKind {
    File,
    Directory,
}

impl StorageEntryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

/// The storage handler bound to one plugin's data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStorage {
    data_dir: PathBuf,
}

impl PluginStorage {
    /// Binds the handler to the data directory the lifecycle resolved for the launched plugin.
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Lists the entries directly below `path`; the root hides `web-profile/`.
    pub fn list(&self, path: &PortableRelativePath) -> Result<Vec<StorageEntry>, StorageError> {
        let directory = self.resolve_existing(path)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&directory).map_err(StorageError::from_io)? {
            let entry = entry.map_err(StorageError::from_io)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if path.is_root() && name == WEB_PROFILE_DIRECTORY {
                continue;
            }
            // `file_type` does not follow links, so a symlink is reported as such and skipped
            // rather than classified by whatever it currently points at.
            let file_type = entry.file_type().map_err(StorageError::from_io)?;
            let kind = if file_type.is_file() {
                StorageEntryKind::File
            } else if file_type.is_dir() {
                StorageEntryKind::Directory
            } else {
                continue;
            };
            let size_bytes = match kind {
                StorageEntryKind::File => entry.metadata().map_err(StorageError::from_io)?.len(),
                StorageEntryKind::Directory => 0,
            };
            entries.push(StorageEntry {
                name,
                kind,
                size_bytes,
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    /// Reads one regular file, refusing anything above `MAX_STORAGE_FILE_BYTES`.
    pub fn read(&self, path: &PortableRelativePath) -> Result<Vec<u8>, StorageError> {
        let file = self.resolve_existing(path)?;
        let metadata = fs::metadata(&file).map_err(StorageError::from_io)?;
        if !metadata.is_file() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidPath,
                "path is not a regular file",
            ));
        }
        if metadata.len() > MAX_STORAGE_FILE_BYTES {
            return Err(StorageError::new(
                StorageErrorKind::TooLarge,
                format!("file exceeds {MAX_STORAGE_FILE_BYTES} bytes"),
            ));
        }
        fs::read(&file).map_err(StorageError::from_io)
    }

    /// Atomically replaces one file, creating missing parent directories inside the data dir.
    pub fn write(&self, path: &PortableRelativePath, bytes: &[u8]) -> Result<(), StorageError> {
        if path.is_root() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidPath,
                "cannot write to the data directory itself",
            ));
        }
        if bytes.len() as u64 > MAX_STORAGE_FILE_BYTES {
            return Err(StorageError::new(
                StorageErrorKind::TooLarge,
                format!("content exceeds {MAX_STORAGE_FILE_BYTES} bytes"),
            ));
        }
        let root = self.root()?;
        let target = root.as_path().join(path.to_path_buf());
        // The target may not exist yet, so containment is checked on the longest existing
        // prefix: a symlink anywhere on the way would canonicalize elsewhere and fail this test.
        if canonicalize_longest_existing_prefix(&target) != target {
            return Err(StorageError::new(
                StorageErrorKind::InvalidPath,
                "path traverses a symlink or escapes the plugin data directory",
            ));
        }
        // A non-root logical path always has a parent (at least the data directory itself).
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(StorageError::from_io)?;
        }
        ora_utils::atomic::write(&target, bytes).map_err(StorageError::from_io)
    }

    /// Removes one file or one directory tree.
    pub fn remove(&self, path: &PortableRelativePath) -> Result<(), StorageError> {
        if path.is_root() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidPath,
                "cannot remove the data directory itself",
            ));
        }
        let target = self.resolve_existing(path)?;
        let metadata = fs::metadata(&target).map_err(StorageError::from_io)?;
        if metadata.is_dir() {
            fs::remove_dir_all(&target).map_err(StorageError::from_io)
        } else {
            fs::remove_file(&target).map_err(StorageError::from_io)
        }
    }

    /// Canonicalizes the data directory; it is created at launch, so failure here is an I/O fault.
    fn root(&self) -> Result<CanonicalPathRoot, StorageError> {
        CanonicalPathRoot::new(&self.data_dir).map_err(StorageError::from_containment)
    }

    /// Resolves an existing logical path and requires it to be reached without any symlink.
    ///
    /// The root is canonical and the logical path has no `.`/`..`, so the canonical target can
    /// only differ from the lexical join when a symlink sits on the path. Symlinks are not part
    /// of the logical namespace: even one that currently points inside the directory could be
    /// retargeted later, so the whole class is refused rather than checked per call.
    fn resolve_existing(&self, path: &PortableRelativePath) -> Result<PathBuf, StorageError> {
        let root = self.root()?;
        let resolved = root
            .resolve_existing(path)
            .map_err(StorageError::from_containment)?;
        if resolved != root.as_path().join(path.to_path_buf()) {
            return Err(StorageError::new(
                StorageErrorKind::InvalidPath,
                "path traverses a symlink",
            ));
        }
        Ok(resolved)
    }
}

impl HostRequestHandler for PluginStorage {
    /// Parses params, runs the blocking filesystem work off the async executor, and renders the
    /// documented result shapes.
    async fn handle(&self, method: &str, params: Value) -> Result<Value, HostRequestError> {
        let operation = match method {
            STORAGE_LIST_METHOD => StorageOperation::List,
            STORAGE_READ_METHOD => StorageOperation::Read,
            STORAGE_WRITE_METHOD => StorageOperation::Write,
            STORAGE_REMOVE_METHOD => StorageOperation::Remove,
            other => return Err(HostRequestError::method_not_found(other)),
        };
        let path = logical_path(&params)?;
        let bytes = match operation {
            StorageOperation::Write => Some(decode_bytes(&params)?),
            StorageOperation::List | StorageOperation::Read | StorageOperation::Remove => None,
        };
        let storage = self.clone();
        let result = tokio::task::spawn_blocking(move || match operation {
            StorageOperation::List => storage.list(&path).map(|entries| {
                json!({
                    "entries": entries
                        .iter()
                        .map(|entry| json!({
                            "name": entry.name,
                            "kind": entry.kind.as_str(),
                            "size_bytes": entry.size_bytes,
                        }))
                        .collect::<Vec<_>>(),
                })
            }),
            StorageOperation::Read => storage
                .read(&path)
                .map(|bytes| json!({ "bytes_base64": BASE64.encode(bytes) })),
            StorageOperation::Write => storage
                .write(&path, &bytes.unwrap_or_default())
                .map(|()| json!({})),
            StorageOperation::Remove => storage.remove(&path).map(|()| json!({})),
        })
        .await
        .map_err(|error| StorageError::new(StorageErrorKind::Io, error.to_string()))?;
        result.map_err(HostRequestError::from)
    }
}

/// The four storage methods, resolved once so the blocking closure matches exhaustively.
#[derive(Debug, Clone, Copy)]
enum StorageOperation {
    List,
    Read,
    Write,
    Remove,
}

/// Extracts and validates the logical `path` param; `web-profile` is refused at the first segment.
fn logical_path(params: &Value) -> Result<PortableRelativePath, StorageError> {
    let raw = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::new(StorageErrorKind::InvalidParams, "missing string path"))?;
    let path = PortableRelativePath::parse(raw)
        .map_err(|error| StorageError::new(StorageErrorKind::InvalidPath, error.to_string()))?;
    if path.as_str().split('/').next() == Some(WEB_PROFILE_DIRECTORY) {
        return Err(StorageError::new(
            StorageErrorKind::InvalidPath,
            "web-profile is owned by the host and not exposed",
        ));
    }
    Ok(path)
}

/// Extracts and decodes the `bytes_base64` param of a write.
fn decode_bytes(params: &Value) -> Result<Vec<u8>, StorageError> {
    let encoded = params
        .get("bytes_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            StorageError::new(
                StorageErrorKind::InvalidParams,
                "missing string bytes_base64",
            )
        })?;
    BASE64.decode(encoded).map_err(|error| {
        StorageError::new(
            StorageErrorKind::InvalidParams,
            format!("bytes_base64 is not valid base64: {error}"),
        )
    })
}

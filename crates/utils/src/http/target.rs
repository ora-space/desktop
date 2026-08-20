//! Temporary-file staging helpers shared by every download backend.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use super::error::DownloadError;

/// Fallback file name for a destination that has none.
pub(super) const DEFAULT_FILE_NAME: &str = "download";

/// Returns the `.tmp` sibling used to stage a download before the atomic rename.
pub(super) fn temporary_sibling(destination: &Path) -> PathBuf {
    let parent = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut file_name = destination
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from(DEFAULT_FILE_NAME));
    file_name.push(".tmp");
    parent.join(file_name)
}

/// Wraps an I/O failure with the path that caused it.
pub(super) fn io_error(path: &Path, error: io::Error) -> DownloadError {
    DownloadError::Io {
        path: path.to_path_buf(),
        source: error,
    }
}

/// Atomically renames `temporary` over `destination` on the same filesystem.
pub(super) fn rename_over(destination: &Path, temporary: &Path) -> Result<(), DownloadError> {
    std::fs::rename(temporary, destination).map_err(|error| io_error(destination, error))
}

/// Removes a leftover temporary file, ignoring failures so cleanup never masks the original error.
pub(super) fn remove_temporary(temporary: &Path) {
    let _ = std::fs::remove_file(temporary);
}

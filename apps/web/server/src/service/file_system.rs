use axum::http::StatusCode;
use ora_contracts::{
    FileSystemBreadcrumb, FileSystemEntry, FileSystemEntryKind, ListDirectoryRequest,
    ListDirectoryResponse,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::error::WebApiError;

/// Provides read-only directory metadata for the web platform path picker.
pub struct FileSystemApi {
    home_directory: PathBuf,
}

impl FileSystemApi {
    /// Creates a filesystem API whose omitted listing path resolves to the supplied home directory.
    pub fn new(home_directory: PathBuf) -> Self {
        Self { home_directory }
    }

    /// Lists one absolute directory while preserving user-visible symbolic-link paths.
    pub fn list_directory(
        &self,
        request: ListDirectoryRequest,
    ) -> Result<ListDirectoryResponse, FileSystemError> {
        let directory = request
            .path
            .map_or_else(|| self.home_directory.clone(), PathBuf::from);

        if !directory.is_absolute() {
            return Err(FileSystemError::PathNotAbsolute { path: directory });
        }

        let directory_metadata =
            fs::metadata(&directory).map_err(|source| FileSystemError::DirectoryRead {
                path: directory.clone(),
                source,
            })?;
        if !directory_metadata.is_dir() {
            return Err(FileSystemError::NotDirectory { path: directory });
        }

        let read_directory =
            fs::read_dir(&directory).map_err(|source| FileSystemError::DirectoryRead {
                path: directory.clone(),
                source,
            })?;
        let mut entries = Vec::new();

        for entry in read_directory {
            let entry = entry.map_err(|source| FileSystemError::DirectoryRead {
                path: directory.clone(),
                source,
            })?;
            entries.push(to_contract_entry(entry));
        }

        // Cached keys keep sorting deterministic without repeatedly allocating lowercase names.
        entries.sort_by_cached_key(|entry| {
            (
                entry_kind_rank(entry.kind),
                entry.name.to_lowercase(),
                entry.name.clone(),
            )
        });

        Ok(ListDirectoryResponse {
            current_path: path_to_string(&directory),
            parent_path: directory.parent().map(path_to_string),
            breadcrumbs: directory_breadcrumbs(&directory),
            entries,
        })
    }
}

/// Describes filesystem failures that need stable HTTP status and error-code mappings.
#[derive(Debug, Error)]
pub enum FileSystemError {
    #[error("filesystem path must be absolute: {path:?}")]
    PathNotAbsolute { path: PathBuf },
    #[error("filesystem path is not a directory: {path:?}")]
    NotDirectory { path: PathBuf },
    #[error("failed to read filesystem directory {path:?}")]
    DirectoryRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl From<FileSystemError> for WebApiError {
    /// Maps directory browsing failures into stable HTTP responses for the Web picker.
    fn from(error: FileSystemError) -> Self {
        match error {
            FileSystemError::PathNotAbsolute { path } => WebApiError::file_system(
                StatusCode::BAD_REQUEST,
                "invalid_file_system_path",
                format!("filesystem path must be absolute: {path:?}"),
            ),
            FileSystemError::NotDirectory { path } => WebApiError::file_system(
                StatusCode::BAD_REQUEST,
                "file_system_path_not_directory",
                format!("filesystem path is not a directory: {path:?}"),
            ),
            FileSystemError::DirectoryRead { path, source }
                if source.kind() == io::ErrorKind::NotFound =>
            {
                WebApiError::file_system(
                    StatusCode::NOT_FOUND,
                    "file_system_path_not_found",
                    format!("filesystem path was not found: {path:?}"),
                )
            }
            FileSystemError::DirectoryRead { path, source }
                if source.kind() == io::ErrorKind::PermissionDenied =>
            {
                WebApiError::file_system(
                    StatusCode::FORBIDDEN,
                    "file_system_directory_unreadable",
                    format!("filesystem directory is not readable: {path:?}"),
                )
            }
            FileSystemError::DirectoryRead { path, .. } => WebApiError::file_system(
                StatusCode::INTERNAL_SERVER_ERROR,
                "file_system_directory_read_failed",
                format!("failed to read filesystem directory: {path:?}"),
            ),
        }
    }
}

/// Converts one directory entry while keeping broken links visible but unavailable.
fn to_contract_entry(entry: fs::DirEntry) -> FileSystemEntry {
    let path = entry.path();
    let link_metadata = fs::symlink_metadata(&path);
    let is_symbolic_link = link_metadata
        .as_ref()
        .is_ok_and(|metadata| metadata.file_type().is_symlink());
    let resolved_metadata = if is_symbolic_link {
        fs::metadata(&path)
    } else {
        link_metadata
    };
    let kind = match resolved_metadata {
        Ok(metadata) if metadata.is_dir() => FileSystemEntryKind::Directory,
        Ok(metadata) if metadata.is_file() => FileSystemEntryKind::File,
        Ok(_) | Err(_) => FileSystemEntryKind::Unavailable,
    };

    FileSystemEntry {
        name: entry.file_name().to_string_lossy().into_owned(),
        path: path_to_string(&path),
        kind,
        is_symbolic_link,
    }
}

/// Provides a stable directory-before-file sort order for the picker.
fn entry_kind_rank(kind: FileSystemEntryKind) -> u8 {
    match kind {
        FileSystemEntryKind::Directory => 0,
        FileSystemEntryKind::File => 1,
        FileSystemEntryKind::Unavailable => 2,
    }
}

/// Preserves platform-native path spelling across the JSON adapter boundary.
fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Builds clickable ancestors on the server so the browser never guesses native path separators.
fn directory_breadcrumbs(directory: &Path) -> Vec<FileSystemBreadcrumb> {
    let mut breadcrumbs = directory
        .ancestors()
        .map(|path| FileSystemBreadcrumb {
            name: path.file_name().map_or_else(
                || path_to_string(path),
                |name| name.to_string_lossy().into_owned(),
            ),
            path: path_to_string(path),
        })
        .collect::<Vec<_>>();
    breadcrumbs.reverse();
    breadcrumbs
}

#[cfg(test)]
mod tests {
    use super::FileSystemApi;
    use ora_contracts::{FileSystemEntry, FileSystemEntryKind, ListDirectoryRequest};
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Verifies omitted paths start at home and include hidden entries in deterministic order.
    #[test]
    fn lists_home_with_directories_before_files() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join("z-directory")).unwrap();
        fs::create_dir(home.path().join(".hidden")).unwrap();
        fs::write(home.path().join("A-file"), "fixture").unwrap();
        let api = FileSystemApi::new(home.path().to_path_buf());

        let response = api
            .list_directory(ListDirectoryRequest::default())
            .unwrap_or_else(|error| panic!("expected directory listing: {error}"));

        assert_eq!(
            response.entries,
            vec![
                FileSystemEntry {
                    name: ".hidden".to_string(),
                    path: home.path().join(".hidden").to_string_lossy().to_string(),
                    kind: FileSystemEntryKind::Directory,
                    is_symbolic_link: false,
                },
                FileSystemEntry {
                    name: "z-directory".to_string(),
                    path: home
                        .path()
                        .join("z-directory")
                        .to_string_lossy()
                        .to_string(),
                    kind: FileSystemEntryKind::Directory,
                    is_symbolic_link: false,
                },
                FileSystemEntry {
                    name: "A-file".to_string(),
                    path: home.path().join("A-file").to_string_lossy().to_string(),
                    kind: FileSystemEntryKind::File,
                    is_symbolic_link: false,
                },
            ]
        );
    }

    /// Verifies relative browsing paths are rejected instead of depending on the server cwd.
    #[test]
    fn rejects_relative_paths() {
        let home = TempDir::new().unwrap();
        let api = FileSystemApi::new(home.path().to_path_buf());

        let result = api.list_directory(ListDirectoryRequest {
            path: Some("relative".to_string()),
        });

        assert!(matches!(
            result,
            Err(super::FileSystemError::PathNotAbsolute { .. })
        ));
    }

    /// Verifies broken symbolic links remain visible as unavailable entries.
    #[cfg(unix)]
    #[test]
    fn preserves_broken_symbolic_links() {
        use std::os::unix::fs::symlink;

        let home = TempDir::new().unwrap();
        symlink(home.path().join("missing"), home.path().join("broken")).unwrap();
        let api = FileSystemApi::new(home.path().to_path_buf());

        let response = api
            .list_directory(ListDirectoryRequest::default())
            .unwrap_or_else(|error| panic!("expected directory listing: {error}"));

        assert_eq!(
            response.entries,
            vec![FileSystemEntry {
                name: "broken".to_string(),
                path: home.path().join("broken").to_string_lossy().to_string(),
                kind: FileSystemEntryKind::Unavailable,
                is_symbolic_link: true,
            }]
        );
    }
}

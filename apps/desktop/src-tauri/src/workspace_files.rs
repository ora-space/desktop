use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{
    EmptyErrorParams, ListWorkspaceDirectoryResponse, PublicError, ReadWorkspaceFileResponse,
    SearchWorkspaceResponse, WorkspaceEntry, WorkspaceEntryKind, WorkspaceSearchKind,
    WorkspaceSearchResult,
};
use ora_fs::{
    DirectoryEntry, DirectoryEntryKind, SearchKind, SearchResult, WorkspaceFileSystem,
    WorkspaceFileSystemError, WorkspaceWatcher,
};
use std::path::{Path, PathBuf};

/// Adapts the reusable workspace filesystem to Desktop transport values.
pub(crate) struct WorkspaceFileApi {
    file_system: WorkspaceFileSystem,
}

impl WorkspaceFileApi {
    /// Creates the production adapter with the executable used for workspace search.
    pub(crate) fn new(ripgrep_path: PathBuf) -> Self {
        Self {
            file_system: WorkspaceFileSystem::system(ripgrep_path),
        }
    }

    /// Creates one empty file or directory under a workspace-relative path.
    pub(crate) fn create_entry(
        &self,
        root: &Path,
        path: &Path,
        kind: WorkspaceEntryKind,
    ) -> Result<WorkspaceEntry, WorkspaceFileSystemError> {
        let entry = self.file_system.create_entry(
            root,
            path,
            match kind {
                WorkspaceEntryKind::File => DirectoryEntryKind::File,
                WorkspaceEntryKind::Directory => DirectoryEntryKind::Directory,
            },
        )?;
        Ok(map_workspace_entry(entry))
    }

    /// Copies one contained path onto a missing destination.
    pub(crate) fn copy_entry(
        &self,
        root: &Path,
        from: &Path,
        path: &Path,
    ) -> Result<WorkspaceEntry, WorkspaceFileSystemError> {
        Ok(map_workspace_entry(
            self.file_system.copy_entry(root, from, path)?,
        ))
    }

    /// Moves or renames one contained path onto a missing destination.
    pub(crate) fn move_entry(
        &self,
        root: &Path,
        from: &Path,
        path: &Path,
    ) -> Result<WorkspaceEntry, WorkspaceFileSystemError> {
        Ok(map_workspace_entry(
            self.file_system.move_entry(root, from, path)?,
        ))
    }

    /// Deletes one contained file, directory tree, or symlink.
    pub(crate) fn delete_entry(
        &self,
        root: &Path,
        path: &Path,
    ) -> Result<(), WorkspaceFileSystemError> {
        self.file_system.delete_entry(root, path)
    }

    /// Lists one workspace-relative directory after the path has been contained.
    pub(crate) fn list_directory(
        &self,
        root: &Path,
        path: &Path,
    ) -> Result<ListWorkspaceDirectoryResponse, WorkspaceFileSystemError> {
        let listing = self.file_system.list_directory(root, path)?;
        Ok(ListWorkspaceDirectoryResponse {
            path: listing.path,
            entries: listing
                .entries
                .into_iter()
                .map(map_workspace_entry)
                .collect(),
        })
    }

    /// Reads one bounded UTF-8 file from the task workspace.
    pub(crate) fn read_file(
        &self,
        root: &Path,
        path: &Path,
    ) -> Result<ReadWorkspaceFileResponse, WorkspaceFileSystemError> {
        let file = self.file_system.read_file(root, path)?;
        Ok(ReadWorkspaceFileResponse {
            path: file.path,
            content: file.content,
            version: file.version,
            size_bytes: u32::try_from(file.size_bytes).unwrap_or(u32::MAX),
        })
    }

    /// Searches the task workspace with the configured ripgrep process runner.
    pub(crate) async fn search(
        &self,
        root: &Path,
        query: &str,
        kind: WorkspaceSearchKind,
    ) -> Result<SearchWorkspaceResponse, WorkspaceFileSystemError> {
        let search = self
            .file_system
            .search(
                root,
                query,
                match kind {
                    WorkspaceSearchKind::Files => SearchKind::Files,
                    WorkspaceSearchKind::Content => SearchKind::Content,
                },
            )
            .await?;
        Ok(SearchWorkspaceResponse {
            results: search
                .results
                .into_iter()
                .map(|result| match result {
                    SearchResult::File { path } => WorkspaceSearchResult::File { path },
                    SearchResult::Match(found) => WorkspaceSearchResult::Match {
                        path: found.path,
                        line: u32::try_from(found.line).unwrap_or(u32::MAX),
                        column: u32::try_from(found.column).unwrap_or(u32::MAX),
                        matched_text: found.matched_text,
                        preview: found.preview,
                    },
                })
                .collect(),
            truncated: search.truncated,
        })
    }

    /// Starts a recursive native watcher rooted at one task workspace.
    pub(crate) fn watch(&self, root: &Path) -> Result<WorkspaceWatcher, WorkspaceFileSystemError> {
        WorkspaceWatcher::start(root)
    }
}

/// Projects one crate-native listing row onto the Desktop contract value.
fn map_workspace_entry(entry: DirectoryEntry) -> WorkspaceEntry {
    WorkspaceEntry {
        name: entry.name,
        path: entry.path,
        kind: match entry.kind {
            DirectoryEntryKind::File => WorkspaceEntryKind::File,
            DirectoryEntryKind::Directory => WorkspaceEntryKind::Directory,
        },
        is_symbolic_link: entry.is_symbolic_link,
    }
}

/// Projects filesystem failures into stable transport-neutral error classifications.
pub(crate) fn workspace_file_backend_error(error: WorkspaceFileSystemError) -> BackendError {
    let (classification, public_error, context) = match &error {
        WorkspaceFileSystemError::PathNotFound { .. } => (
            ErrorClassification::NotFound,
            PublicError::FileSystemPathNotFound(EmptyErrorParams {}),
            "workspace path was not found",
        ),
        WorkspaceFileSystemError::AlreadyExists { .. } => (
            ErrorClassification::Conflict,
            PublicError::FileSystemPathAlreadyExists(EmptyErrorParams {}),
            "workspace path already exists",
        ),
        WorkspaceFileSystemError::PermissionDenied { .. } => (
            ErrorClassification::Forbidden,
            PublicError::FileSystemPathPermissionDenied(EmptyErrorParams {}),
            "workspace path access was denied",
        ),
        WorkspaceFileSystemError::PathNotRelative { .. }
        | WorkspaceFileSystemError::PathOutsideWorkspace { .. }
        | WorkspaceFileSystemError::NotDirectory { .. }
        | WorkspaceFileSystemError::NotFile { .. }
        | WorkspaceFileSystemError::BinaryFile { .. }
        | WorkspaceFileSystemError::InvalidUtf8 { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "workspace file request is invalid",
        ),
        WorkspaceFileSystemError::FileTooLarge { .. }
        | WorkspaceFileSystemError::SearchOutputTooLarge { .. } => (
            ErrorClassification::PayloadTooLarge,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "workspace output is too large",
        ),
        WorkspaceFileSystemError::SearchTimedOut => (
            ErrorClassification::Unprocessable,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "workspace search timed out",
        ),
        WorkspaceFileSystemError::WorkspaceUnavailable { .. }
        | WorkspaceFileSystemError::Io { .. }
        | WorkspaceFileSystemError::SearchToolUnavailable { .. }
        | WorkspaceFileSystemError::SearchFailed { .. }
        | WorkspaceFileSystemError::InvalidSearchOutput { .. }
        | WorkspaceFileSystemError::WatchFailed { .. } => (
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "workspace filesystem operation failed",
        ),
    };
    BackendError::with_source(classification, public_error, context, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::io;
    use std::path::PathBuf;

    /// A denial is a user-visible access refusal, so it must not surface as an internal fault.
    #[test]
    fn maps_permission_denials_to_a_forbidden_public_error() {
        let error = workspace_file_backend_error(WorkspaceFileSystemError::PermissionDenied {
            path: PathBuf::from("workspace/secret"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        });

        assert_eq!(
            (error.classification(), error.public_error().code()),
            (
                ErrorClassification::Forbidden,
                "file_system_path_permission_denied"
            )
        );
    }

    /// Opaque I/O faults stay internal so a denial's dedicated code keeps its meaning.
    #[test]
    fn keeps_other_io_faults_internal() {
        let error = workspace_file_backend_error(WorkspaceFileSystemError::Io {
            path: PathBuf::from("workspace/file"),
            source: io::Error::from(io::ErrorKind::NotADirectory),
        });

        assert_eq!(
            (error.classification(), error.public_error().code()),
            (ErrorClassification::Internal, "internal_error")
        );
    }

    /// A create that collides with an existing name is a conflict the tree can show.
    #[test]
    fn maps_already_exists_to_a_conflict_public_error() {
        let error = workspace_file_backend_error(WorkspaceFileSystemError::AlreadyExists {
            path: PathBuf::from("workspace/README.md"),
        });

        assert_eq!(
            (error.classification(), error.public_error().code()),
            (
                ErrorClassification::Conflict,
                "file_system_path_already_exists"
            )
        );
    }
}

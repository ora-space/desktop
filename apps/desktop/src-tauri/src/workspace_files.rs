use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{
    EmptyErrorParams, ListWorkspaceDirectoryResponse, PublicError, ReadWorkspaceFileResponse,
    SearchWorkspaceResponse, WorkspaceEntry, WorkspaceEntryKind, WorkspaceSearchKind,
    WorkspaceSearchResult,
};
use ora_fs::{
    DirectoryEntryKind, SearchKind, SearchResult, WorkspaceFileSystem, WorkspaceFileSystemError,
    WorkspaceWatcher,
};
use std::path::{Path, PathBuf};

/// Adapts the reusable read-only workspace filesystem to Desktop transport values.
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

    /// Lists one immediate directory while keeping the task root owned by Desktop.
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
                .map(|entry| WorkspaceEntry {
                    name: entry.name,
                    path: entry.path,
                    kind: match entry.kind {
                        DirectoryEntryKind::File => WorkspaceEntryKind::File,
                        DirectoryEntryKind::Directory => WorkspaceEntryKind::Directory,
                    },
                    is_symbolic_link: entry.is_symbolic_link,
                })
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

/// Projects filesystem failures into stable transport-neutral error classifications.
pub(crate) fn workspace_file_backend_error(error: WorkspaceFileSystemError) -> BackendError {
    let (classification, public_error, context) = match &error {
        WorkspaceFileSystemError::PathNotFound { .. } => (
            ErrorClassification::NotFound,
            PublicError::FileSystemPathNotFound(EmptyErrorParams {}),
            "workspace path was not found",
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

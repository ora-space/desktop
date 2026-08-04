use ora_contracts::{
    ListWorkspaceDirectoryResponse, ReadWorkspaceFileResponse, SearchWorkspaceResponse,
    WorkspaceEntry, WorkspaceEntryKind, WorkspaceSearchKind, WorkspaceSearchResult,
};
use ora_fs::{
    DirectoryEntryKind, SearchKind, SearchResult, WorkspaceFileSystem, WorkspaceFileSystemError,
    WorkspaceWatcher,
};
use std::path::{Path, PathBuf};

/// Adapts the reusable workspace filesystem into transport contract values.
pub struct WorkspaceFileApi {
    file_system: WorkspaceFileSystem,
}

impl WorkspaceFileApi {
    /// Creates the production adapter with the resolved bundled ripgrep executable.
    pub fn new(ripgrep_path: PathBuf) -> Self {
        Self {
            file_system: WorkspaceFileSystem::system(ripgrep_path),
        }
    }

    /// Lists one immediate task-workspace directory.
    pub fn list_directory(
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

    /// Reads one bounded UTF-8 task-workspace file.
    pub fn read_file(
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

    /// Searches one task workspace with the bundled ripgrep executable.
    pub async fn search(
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

    /// Starts a recursive native watcher for one task workspace.
    pub fn watch(&self, root: &Path) -> Result<WorkspaceWatcher, WorkspaceFileSystemError> {
        WorkspaceWatcher::start(root)
    }
}

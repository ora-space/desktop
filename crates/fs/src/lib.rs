mod error;
mod search;
mod spec;
mod watch;
mod workspace;

pub use error::WorkspaceFileSystemError;
pub use search::{SearchKind, SearchMatch, SearchResult, SearchResults};
pub use spec::{MarkdownFile, MarkdownIndex};
pub use watch::{WorkspaceChange, WorkspaceChangeKind, WorkspaceWatcher};
pub use workspace::{
    DirectoryEntry, DirectoryEntryKind, DirectoryListing, ReadFile, WorkspaceFileSystem,
};

use crate::WorkspaceFileSystemError;
use ora_process::TokioProcessSpawner;
use ora_utils::path::{CanonicalPathRoot, PathContainmentError, PortableRelativePath};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Distinguishes the two selectable entry kinds returned by a workspace directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryEntryKind {
    File,
    Directory,
}

/// Describes one workspace-relative directory entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: DirectoryEntryKind,
    pub is_symbolic_link: bool,
}

/// Returns one directory's normalized relative path and immediate children.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryListing {
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
}

/// Carries one UTF-8 text file and a version token derived from its disk metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadFile {
    pub path: String,
    pub content: String,
    pub version: String,
    pub size_bytes: u64,
}

/// Owns workspace-scoped filesystem access and its injected process runner.
pub struct WorkspaceFileSystem<S = TokioProcessSpawner> {
    pub(crate) ripgrep_path: PathBuf,
    pub(crate) process_spawner: S,
    max_file_bytes: u64,
}

impl WorkspaceFileSystem<TokioProcessSpawner> {
    /// Creates the production filesystem using the supplied bundled ripgrep executable.
    pub fn system(ripgrep_path: PathBuf) -> Self {
        Self::new(ripgrep_path, TokioProcessSpawner::new())
    }
}

impl<S> WorkspaceFileSystem<S> {
    /// Creates a filesystem with an injected process spawner so search behavior remains testable.
    pub fn new(ripgrep_path: PathBuf, process_spawner: S) -> Self {
        Self {
            ripgrep_path,
            process_spawner,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }

    /// Lists one immediate directory while keeping all returned paths relative to the workspace.
    pub fn list_directory(
        &self,
        root: &Path,
        relative_path: &Path,
    ) -> Result<DirectoryListing, WorkspaceFileSystemError> {
        let root = canonical_root(root)?;
        let resolved = resolve_existing(&root, relative_path)?;
        if !resolved.is_dir() {
            return Err(WorkspaceFileSystemError::NotDirectory { path: resolved });
        }

        let mut entries = Vec::new();
        // Canonical containment describes the checked link topology only; read_dir cannot retain a
        // race-proof handle if an untrusted process replaces a symlink after resolution.
        let directory = fs::read_dir(&resolved).map_err(|source| WorkspaceFileSystemError::Io {
            path: resolved.clone(),
            source,
        })?;
        for entry in directory {
            let entry = entry.map_err(|source| WorkspaceFileSystemError::Io {
                path: resolved.clone(),
                source,
            })?;
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let link_metadata =
                fs::symlink_metadata(&path).map_err(|source| WorkspaceFileSystemError::Io {
                    path: path.clone(),
                    source,
                })?;
            let is_symbolic_link = link_metadata.file_type().is_symlink();
            let metadata = if is_symbolic_link {
                fs::metadata(&path).map_err(|source| WorkspaceFileSystemError::Io {
                    path: path.clone(),
                    source,
                })?
            } else {
                link_metadata
            };
            let kind = if metadata.is_dir() {
                DirectoryEntryKind::Directory
            } else if metadata.is_file() {
                DirectoryEntryKind::File
            } else {
                continue;
            };
            let canonical_path =
                path.canonicalize()
                    .map_err(|source| WorkspaceFileSystemError::Io {
                        path: path.clone(),
                        source,
                    })?;
            let relative_path = match relative_string(&root, &canonical_path) {
                Ok(relative_path) => relative_path,
                Err(WorkspaceFileSystemError::PathOutsideWorkspace { .. }) => continue,
                Err(error) => return Err(error),
            };
            entries.push(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: relative_path,
                kind,
                is_symbolic_link,
            });
        }

        entries.sort_by_cached_key(|entry| {
            (
                match entry.kind {
                    DirectoryEntryKind::Directory => 0,
                    DirectoryEntryKind::File => 1,
                },
                entry.name.to_lowercase(),
                entry.name.clone(),
            )
        });

        Ok(DirectoryListing {
            path: relative_string(&root, &resolved)?,
            entries,
        })
    }

    /// Reads one bounded UTF-8 text file without allowing traversal or symlink escape.
    pub fn read_file(
        &self,
        root: &Path,
        relative_path: &Path,
    ) -> Result<ReadFile, WorkspaceFileSystemError> {
        let root = canonical_root(root)?;
        let resolved = resolve_existing(&root, relative_path)?;
        // Canonical containment rejects the current symlink target, but these path-based metadata
        // and read calls remain subject to replacement between the check and use (TOCTOU).
        let metadata = fs::metadata(&resolved).map_err(|source| WorkspaceFileSystemError::Io {
            path: resolved.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(WorkspaceFileSystemError::NotFile { path: resolved });
        }
        if metadata.len() > self.max_file_bytes {
            return Err(WorkspaceFileSystemError::FileTooLarge {
                path: resolved,
                limit_bytes: self.max_file_bytes,
            });
        }

        let bytes = fs::read(&resolved).map_err(|source| WorkspaceFileSystemError::Io {
            path: resolved.clone(),
            source,
        })?;
        if bytes.contains(&0) {
            return Err(WorkspaceFileSystemError::BinaryFile { path: resolved });
        }
        let content =
            String::from_utf8(bytes).map_err(|_| WorkspaceFileSystemError::InvalidUtf8 {
                path: resolved.clone(),
            })?;
        let modified_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_millis());
        Ok(ReadFile {
            path: relative_string(&root, &resolved)?,
            content,
            version: format!("{modified_millis}:{}", metadata.len()),
            size_bytes: metadata.len(),
        })
    }
}

/// Resolves one existing relative path and verifies its canonical target remains under the root.
pub(crate) fn resolve_existing(
    root: &CanonicalPathRoot,
    relative_path: &Path,
) -> Result<PathBuf, WorkspaceFileSystemError> {
    let relative_path = parse_relative(relative_path)?;
    root.resolve_existing(&relative_path)
        .map_err(map_containment_error)
}

/// Canonicalizes the workspace root so every later containment check uses one stable spelling.
pub(crate) fn canonical_root(root: &Path) -> Result<CanonicalPathRoot, WorkspaceFileSystemError> {
    CanonicalPathRoot::new(root).map_err(map_containment_error)
}

/// Converts a canonical workspace path to the slash-separated wire representation.
pub(crate) fn relative_string(
    root: &CanonicalPathRoot,
    path: &Path,
) -> Result<String, WorkspaceFileSystemError> {
    root.relative_path(path)
        .map(|relative| relative.as_str().to_string())
        .map_err(map_containment_error)
}

/// Parses one UTF-8 request path with the shared platform-independent safety rules.
fn parse_relative(path: &Path) -> Result<PortableRelativePath, WorkspaceFileSystemError> {
    let value = path
        .to_str()
        .ok_or_else(|| WorkspaceFileSystemError::PathNotRelative {
            path: path.to_path_buf(),
        })?;
    PortableRelativePath::parse(value).map_err(|_| WorkspaceFileSystemError::PathNotRelative {
        path: path.to_path_buf(),
    })
}

/// Maps generic containment failures into the stable workspace filesystem error surface.
pub(crate) fn map_containment_error(error: PathContainmentError) -> WorkspaceFileSystemError {
    match error {
        PathContainmentError::RootUnavailable { path, source } => {
            WorkspaceFileSystemError::WorkspaceUnavailable { path, source }
        }
        PathContainmentError::PathNotAbsolute { path }
        | PathContainmentError::NonUtf8Path { path }
        | PathContainmentError::NonPortablePath { path }
        | PathContainmentError::NonCanonicalPath { path } => {
            WorkspaceFileSystemError::PathNotRelative { path }
        }
        PathContainmentError::PathNotFound { path } => {
            WorkspaceFileSystemError::PathNotFound { path }
        }
        PathContainmentError::OutsideRoot { path } => {
            WorkspaceFileSystemError::PathOutsideWorkspace { path }
        }
        PathContainmentError::Io { path, source } => WorkspaceFileSystemError::Io { path, source },
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryEntry, DirectoryEntryKind, WorkspaceFileSystem};
    use ora_process::TokioProcessSpawner;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Verifies listings stay relative, hide Git internals, and sort directories before files.
    #[test]
    fn lists_workspace_directory() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::create_dir(workspace.path().join("src"))
            .unwrap_or_else(|error| panic!("create source directory: {error}"));
        fs::create_dir(workspace.path().join(".git"))
            .unwrap_or_else(|error| panic!("create Git fixture directory: {error}"));
        fs::write(workspace.path().join("README.md"), "hello")
            .unwrap_or_else(|error| panic!("write README fixture: {error}"));
        let file_system = WorkspaceFileSystem::new("rg".into(), TokioProcessSpawner::new());

        let listing = file_system
            .list_directory(workspace.path(), Path::new(""))
            .unwrap_or_else(|error| panic!("list workspace root: {error}"));

        assert_eq!(
            listing.entries,
            vec![
                DirectoryEntry {
                    name: "src".to_string(),
                    path: "src".to_string(),
                    kind: DirectoryEntryKind::Directory,
                    is_symbolic_link: false,
                },
                DirectoryEntry {
                    name: "README.md".to_string(),
                    path: "README.md".to_string(),
                    kind: DirectoryEntryKind::File,
                    is_symbolic_link: false,
                },
            ]
        );
    }

    /// Verifies UTF-8 reads include a metadata-derived version token.
    #[test]
    fn reads_workspace_file() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("main.rs"), "fn main() {}\n")
            .unwrap_or_else(|error| panic!("write Rust fixture: {error}"));
        let file_system = WorkspaceFileSystem::new("rg".into(), TokioProcessSpawner::new());

        let file = file_system
            .read_file(workspace.path(), Path::new("main.rs"))
            .unwrap_or_else(|error| panic!("read workspace file: {error}"));

        assert_eq!(file.path, "main.rs");
        assert_eq!(file.content, "fn main() {}\n");
        assert_eq!(file.size_bytes, 13);
        assert!(file.version.ends_with(":13"));
    }

    /// Verifies parent traversal is rejected before touching the host filesystem.
    #[test]
    fn rejects_parent_traversal() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        let file_system = WorkspaceFileSystem::new("rg".into(), TokioProcessSpawner::new());

        let error = match file_system.read_file(workspace.path(), Path::new("../secret")) {
            Ok(_) => panic!("expected parent traversal to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            super::WorkspaceFileSystemError::PathNotRelative { .. }
        ));
    }
}

use super::entry_dest::join_new_destination_under;
use super::error::WorkspaceFileSystemError;
use super::workspace::WorkspaceFileSystem;
use std::fs;
use std::path::Path;

impl<S> WorkspaceFileSystem<S> {
    /// Deletes one contained file, directory tree, or symlink without following links.
    ///
    /// The workspace root itself is rejected. Directory trees are walked with
    /// `symlink_metadata` so a link inside the tree is unlinked rather than traversed.
    pub fn delete_entry(
        &self,
        root: &Path,
        relative_path: &Path,
    ) -> Result<(), WorkspaceFileSystemError> {
        let (root, destination) = join_new_destination_under(root, relative_path)?;
        if destination.path.as_path() == root.as_path() {
            return Err(WorkspaceFileSystemError::PathOutsideWorkspace {
                path: destination.path,
            });
        }
        remove_tree(&destination.path)
    }
}

/// Removes one contained leaf, unlinking symlinks instead of walking through them.
fn remove_tree(path: &Path) -> Result<(), WorkspaceFileSystemError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            WorkspaceFileSystemError::PathNotFound {
                path: path.to_path_buf(),
            }
        } else {
            WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return remove_symlink(path);
    }
    if metadata.is_file() {
        return fs::remove_file(path)
            .map_err(|source| WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source));
    }
    if !metadata.is_dir() {
        return Err(WorkspaceFileSystemError::NotFile {
            path: path.to_path_buf(),
        });
    }
    let entries = fs::read_dir(path)
        .map_err(|source| WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source))?;
    for entry in entries {
        let entry = entry
            .map_err(|source| WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source))?;
        let child = entry.path();
        if !child.starts_with(path) {
            return Err(WorkspaceFileSystemError::PathOutsideWorkspace { path: child });
        }
        remove_tree(&child)?;
    }
    fs::remove_dir(path)
        .map_err(|source| WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source))
}

/// Unlinks a symlink so a directory junction cannot pull deletion outside the workspace.
fn remove_symlink(path: &Path) -> Result<(), WorkspaceFileSystemError> {
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    fs::remove_dir(path)
        .map_err(|source| WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source))
}

#[cfg(test)]
mod tests {
    use super::super::workspace::WorkspaceFileSystem;
    use super::WorkspaceFileSystemError;
    use ora_process::TokioProcessSpawner;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn file_system() -> WorkspaceFileSystem<TokioProcessSpawner> {
        WorkspaceFileSystem::new("rg".into(), TokioProcessSpawner::new())
    }

    /// A deleted file is gone from the parent directory.
    #[test]
    fn deletes_a_file() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("gone.txt"), "x")
            .unwrap_or_else(|error| panic!("write fixture: {error}"));

        file_system()
            .delete_entry(workspace.path(), Path::new("gone.txt"))
            .unwrap_or_else(|error| panic!("delete file: {error}"));

        assert!(!workspace.path().join("gone.txt").exists());
    }

    /// Directory delete removes nested files as well as the folder.
    #[test]
    fn deletes_a_directory_tree() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::create_dir(workspace.path().join("src"))
            .unwrap_or_else(|error| panic!("create directory: {error}"));
        fs::write(workspace.path().join("src").join("lib.rs"), "mod lib;")
            .unwrap_or_else(|error| panic!("write nested file: {error}"));

        file_system()
            .delete_entry(workspace.path(), Path::new("src"))
            .unwrap_or_else(|error| panic!("delete directory: {error}"));

        assert!(!workspace.path().join("src").exists());
    }

    /// Parent traversal is rejected before anything is removed.
    #[test]
    fn rejects_parent_traversal() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("keep.txt"), "keep")
            .unwrap_or_else(|error| panic!("write fixture: {error}"));

        let error = match file_system().delete_entry(workspace.path(), Path::new("../keep.txt")) {
            Ok(()) => panic!("expected parent traversal to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WorkspaceFileSystemError::PathNotRelative { .. }
        ));
        assert!(workspace.path().join("keep.txt").is_file());
    }

    /// An empty relative path is rejected so the workspace root cannot be removed.
    #[test]
    fn rejects_the_workspace_root() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("keep.txt"), "keep")
            .unwrap_or_else(|error| panic!("write fixture: {error}"));

        let error = match file_system().delete_entry(workspace.path(), Path::new("")) {
            Ok(()) => panic!("expected empty path to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WorkspaceFileSystemError::PathNotRelative { .. }
        ));
        assert!(workspace.path().join("keep.txt").is_file());
    }
}

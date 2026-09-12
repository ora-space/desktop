use super::entry_dest::{join_new_destination_under, map_conflict_io, reject_nested_destination};
use super::error::WorkspaceFileSystemError;
use super::workspace::{
    DirectoryEntry, DirectoryEntryKind, WorkspaceFileSystem, relative_string, resolve_existing,
};
use std::fs;
use std::path::{Path, PathBuf};

impl<S> WorkspaceFileSystem<S> {
    /// Copies one contained file or directory tree onto a missing destination path.
    pub fn copy_entry(
        &self,
        root: &Path,
        from: &Path,
        to: &Path,
    ) -> Result<DirectoryEntry, WorkspaceFileSystemError> {
        relocate_entry(root, from, to, RelocateMode::Copy)
    }

    /// Moves or renames one contained path onto a missing destination path.
    pub fn move_entry(
        &self,
        root: &Path,
        from: &Path,
        to: &Path,
    ) -> Result<DirectoryEntry, WorkspaceFileSystemError> {
        relocate_entry(root, from, to, RelocateMode::Move)
    }
}

enum RelocateMode {
    Copy,
    Move,
}

/// Copies or moves one existing workspace path onto a new contained destination.
fn relocate_entry(
    root: &Path,
    from: &Path,
    to: &Path,
    mode: RelocateMode,
) -> Result<DirectoryEntry, WorkspaceFileSystemError> {
    let (root, destination) = join_new_destination_under(root, to)?;
    let source = resolve_existing(&root, from)?;
    reject_nested_destination(&source, &destination.path)?;
    if destination.path.exists() {
        let existing = destination.path.canonicalize().map_err(|source_io| {
            WorkspaceFileSystemError::from_path_io(destination.path.clone(), source_io)
        })?;
        if existing == source {
            return describe_existing(&root, source);
        }
        return Err(WorkspaceFileSystemError::AlreadyExists {
            path: destination.path,
        });
    }
    let kind = entry_kind(&source)?;
    match mode {
        RelocateMode::Copy => copy_path(&source, &destination.path, kind)?,
        RelocateMode::Move => {
            fs::rename(&source, &destination.path)
                .map_err(|source_io| map_conflict_io(destination.path.clone(), source_io))?;
        }
    }
    let canonical = destination.path.canonicalize().map_err(|source_io| {
        WorkspaceFileSystemError::from_path_io(destination.path.clone(), source_io)
    })?;
    Ok(DirectoryEntry {
        name: destination.name,
        path: relative_string(&root, &canonical)?,
        kind,
        is_symbolic_link: false,
    })
}

/// Reads the kind of an already-contained path after a no-op rename onto itself.
fn describe_existing(
    root: &ora_utils::path::CanonicalPathRoot,
    source: PathBuf,
) -> Result<DirectoryEntry, WorkspaceFileSystemError> {
    let kind = entry_kind(&source)?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkspaceFileSystemError::PathNotRelative {
            path: source.clone(),
        })?
        .to_string();
    Ok(DirectoryEntry {
        name,
        path: relative_string(root, &source)?,
        kind,
        is_symbolic_link: false,
    })
}

/// Distinguishes files from directories without following a later replacement of the source.
fn entry_kind(path: &Path) -> Result<DirectoryEntryKind, WorkspaceFileSystemError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| WorkspaceFileSystemError::from_path_io(path.to_path_buf(), source))?;
    if metadata.file_type().is_symlink() {
        return Err(WorkspaceFileSystemError::PathNotRelative {
            path: path.to_path_buf(),
        });
    }
    if metadata.is_dir() {
        Ok(DirectoryEntryKind::Directory)
    } else if metadata.is_file() {
        Ok(DirectoryEntryKind::File)
    } else {
        Err(WorkspaceFileSystemError::NotFile {
            path: path.to_path_buf(),
        })
    }
}

/// Copies one file or a directory tree without following symbolic links.
fn copy_path(
    source: &Path,
    destination: &Path,
    kind: DirectoryEntryKind,
) -> Result<(), WorkspaceFileSystemError> {
    match kind {
        DirectoryEntryKind::File => {
            fs::copy(source, destination)
                .map_err(|source_io| map_conflict_io(destination.to_path_buf(), source_io))?;
            Ok(())
        }
        DirectoryEntryKind::Directory => copy_directory(source, destination),
    }
}

/// Recursively copies directory children, skipping symbolic links instead of following them.
fn copy_directory(source: &Path, destination: &Path) -> Result<(), WorkspaceFileSystemError> {
    fs::create_dir(destination)
        .map_err(|source_io| map_conflict_io(destination.to_path_buf(), source_io))?;
    let entries = fs::read_dir(source).map_err(|source_io| {
        WorkspaceFileSystemError::from_path_io(source.to_path_buf(), source_io)
    })?;
    for entry in entries {
        let entry = entry.map_err(|source_io| {
            WorkspaceFileSystemError::from_path_io(source.to_path_buf(), source_io)
        })?;
        let child_source = entry.path();
        let name = entry.file_name();
        let child_destination = destination.join(&name);
        if !child_destination.starts_with(destination) {
            return Err(WorkspaceFileSystemError::PathOutsideWorkspace {
                path: child_destination,
            });
        }
        let metadata = fs::symlink_metadata(&child_source).map_err(|source_io| {
            WorkspaceFileSystemError::from_path_io(child_source.clone(), source_io)
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            copy_directory(&child_source, &child_destination)?;
        } else if metadata.is_file() {
            fs::copy(&child_source, &child_destination)
                .map_err(|source_io| map_conflict_io(child_destination, source_io))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::workspace::{DirectoryEntry, DirectoryEntryKind, WorkspaceFileSystem};
    use super::WorkspaceFileSystemError;
    use ora_process::TokioProcessSpawner;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn file_system() -> WorkspaceFileSystem<TokioProcessSpawner> {
        WorkspaceFileSystem::new("rg".into(), TokioProcessSpawner::new())
    }

    /// Copy leaves the source in place and materializes an independent destination file.
    #[test]
    fn copies_a_file_to_a_missing_path() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("a.txt"), "hello")
            .unwrap_or_else(|error| panic!("write source: {error}"));

        let copied = file_system()
            .copy_entry(workspace.path(), Path::new("a.txt"), Path::new("b.txt"))
            .unwrap_or_else(|error| panic!("copy file: {error}"));

        assert_eq!(
            copied,
            DirectoryEntry {
                name: "b.txt".to_string(),
                path: "b.txt".to_string(),
                kind: DirectoryEntryKind::File,
                is_symbolic_link: false,
            }
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("a.txt"))
                .unwrap_or_else(|error| panic!("read source: {error}")),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("b.txt"))
                .unwrap_or_else(|error| panic!("read copy: {error}")),
            "hello"
        );
    }

    /// Directory copy includes nested files and does not remove the source tree.
    #[test]
    fn copies_a_directory_tree() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::create_dir(workspace.path().join("src"))
            .unwrap_or_else(|error| panic!("create source directory: {error}"));
        fs::write(workspace.path().join("src").join("lib.rs"), "mod lib;")
            .unwrap_or_else(|error| panic!("write nested file: {error}"));

        file_system()
            .copy_entry(workspace.path(), Path::new("src"), Path::new("src copy"))
            .unwrap_or_else(|error| panic!("copy directory: {error}"));

        assert_eq!(
            fs::read_to_string(workspace.path().join("src copy").join("lib.rs"))
                .unwrap_or_else(|error| panic!("read copied nested file: {error}")),
            "mod lib;"
        );
        assert!(workspace.path().join("src").join("lib.rs").is_file());
    }

    /// Rename updates the relative path without copying bytes.
    #[test]
    fn renames_a_file_in_place() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("old.rs"), "fn main() {}")
            .unwrap_or_else(|error| panic!("write source: {error}"));

        let moved = file_system()
            .move_entry(workspace.path(), Path::new("old.rs"), Path::new("new.rs"))
            .unwrap_or_else(|error| panic!("rename file: {error}"));

        assert_eq!(moved.path, "new.rs");
        assert!(!workspace.path().join("old.rs").exists());
        assert_eq!(
            fs::read_to_string(workspace.path().join("new.rs"))
                .unwrap_or_else(|error| panic!("read renamed file: {error}")),
            "fn main() {}"
        );
    }

    /// Pasting a folder into itself would recurse forever, so the destination is rejected.
    #[test]
    fn refuses_to_copy_a_directory_into_itself() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::create_dir(workspace.path().join("src"))
            .unwrap_or_else(|error| panic!("create source directory: {error}"));

        let error = match file_system().copy_entry(
            workspace.path(),
            Path::new("src"),
            Path::new("src/nested"),
        ) {
            Ok(_) => panic!("expected a nested copy to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WorkspaceFileSystemError::PathOutsideWorkspace { .. }
        ));
    }
}

use super::entry_dest::{join_new_destination_under, map_conflict_io};
use super::error::WorkspaceFileSystemError;
use super::workspace::{DirectoryEntry, DirectoryEntryKind, WorkspaceFileSystem, relative_string};
use std::fs::{self, File};
use std::path::Path;

impl<S> WorkspaceFileSystem<S> {
    /// Creates one empty file or directory that does not already exist.
    ///
    /// The parent must already be a contained directory. The new path is joined onto that
    /// canonical parent rather than canonicalized first, because the child does not exist yet.
    /// `create_new` / `create_dir` then refuse a race that materialized the same name.
    pub fn create_entry(
        &self,
        root: &Path,
        relative_path: &Path,
        kind: DirectoryEntryKind,
    ) -> Result<DirectoryEntry, WorkspaceFileSystemError> {
        let (root, destination) = join_new_destination_under(root, relative_path)?;
        match kind {
            DirectoryEntryKind::File => {
                File::create_new(&destination.path)
                    .map_err(|source| map_conflict_io(destination.path.clone(), source))?;
            }
            DirectoryEntryKind::Directory => {
                fs::create_dir(&destination.path)
                    .map_err(|source| map_conflict_io(destination.path.clone(), source))?;
            }
        }
        let canonical = destination.path.canonicalize().map_err(|source| {
            WorkspaceFileSystemError::from_path_io(destination.path.clone(), source)
        })?;
        Ok(DirectoryEntry {
            name: destination.name,
            path: relative_string(&root, &canonical)?,
            kind,
            is_symbolic_link: false,
        })
    }
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

    /// An empty file appears in the parent listing and can be read as UTF-8.
    #[test]
    fn creates_an_empty_file_under_an_existing_parent() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::create_dir(workspace.path().join("src"))
            .unwrap_or_else(|error| panic!("create source directory: {error}"));

        let created = file_system()
            .create_entry(
                workspace.path(),
                Path::new("src/lib.rs"),
                DirectoryEntryKind::File,
            )
            .unwrap_or_else(|error| panic!("create file: {error}"));

        assert_eq!(
            created,
            DirectoryEntry {
                name: "lib.rs".to_string(),
                path: "src/lib.rs".to_string(),
                kind: DirectoryEntryKind::File,
                is_symbolic_link: false,
            }
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("src").join("lib.rs"))
                .unwrap_or_else(|error| panic!("read created file: {error}")),
            ""
        );
    }

    /// A directory created from the tree is listable as an empty child folder.
    #[test]
    fn creates_a_directory_under_the_workspace_root() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));

        let created = file_system()
            .create_entry(
                workspace.path(),
                Path::new("docs"),
                DirectoryEntryKind::Directory,
            )
            .unwrap_or_else(|error| panic!("create directory: {error}"));

        assert_eq!(created.kind, DirectoryEntryKind::Directory);
        assert!(workspace.path().join("docs").is_dir());
    }

    /// A second create of the same path must not truncate or replace the first.
    #[test]
    fn refuses_to_replace_an_existing_path() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        fs::write(workspace.path().join("README.md"), "keep")
            .unwrap_or_else(|error| panic!("write fixture: {error}"));

        let error = match file_system().create_entry(
            workspace.path(),
            Path::new("README.md"),
            DirectoryEntryKind::File,
        ) {
            Ok(_) => panic!("expected an existing path to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WorkspaceFileSystemError::AlreadyExists { .. }
        ));
        assert_eq!(
            fs::read_to_string(workspace.path().join("README.md"))
                .unwrap_or_else(|error| panic!("read fixture: {error}")),
            "keep"
        );
    }

    /// Parent traversal is rejected before the host filesystem is asked to create anything.
    #[test]
    fn rejects_parent_traversal() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));

        let error = match file_system().create_entry(
            workspace.path(),
            Path::new("../outside.txt"),
            DirectoryEntryKind::File,
        ) {
            Ok(_) => panic!("expected parent traversal to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            WorkspaceFileSystemError::PathNotRelative { .. }
        ));
    }
}

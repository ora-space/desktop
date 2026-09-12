use super::error::WorkspaceFileSystemError;
use super::workspace::{canonical_root, resolve_existing};
use ora_utils::path::CanonicalPathRoot;
use std::io;
use std::path::{Path, PathBuf};

/// Parent directory plus the single new basename for a contained create/copy/move destination.
pub(crate) struct NewDestination {
    pub name: String,
    pub path: PathBuf,
}

/// Resolves a portable relative destination whose parent already exists inside the workspace.
pub(crate) fn join_new_destination(
    root: &CanonicalPathRoot,
    relative_path: &Path,
) -> Result<NewDestination, WorkspaceFileSystemError> {
    let (parent_relative, name) = split_new_path(relative_path)?;
    let parent = if parent_relative.as_os_str().is_empty() {
        root.as_path().to_path_buf()
    } else {
        resolve_existing(root, &parent_relative)?
    };
    if !parent.is_dir() {
        return Err(WorkspaceFileSystemError::NotDirectory { path: parent });
    }
    let path = parent.join(&name);
    if !path.starts_with(root.as_path()) {
        return Err(WorkspaceFileSystemError::PathOutsideWorkspace { path });
    }
    Ok(NewDestination { name, path })
}

/// Canonicalizes the workspace root then joins one not-yet-created destination.
pub(crate) fn join_new_destination_under(
    root: &Path,
    relative_path: &Path,
) -> Result<(CanonicalPathRoot, NewDestination), WorkspaceFileSystemError> {
    let root = canonical_root(root)?;
    let destination = join_new_destination(&root, relative_path)?;
    Ok((root, destination))
}

/// Rejects copying or moving a directory onto itself or into its own descendant.
pub(crate) fn reject_nested_destination(
    source: &Path,
    destination: &Path,
) -> Result<(), WorkspaceFileSystemError> {
    if destination.starts_with(source) && destination != source {
        return Err(WorkspaceFileSystemError::PathOutsideWorkspace {
            path: destination.to_path_buf(),
        });
    }
    Ok(())
}

/// Maps create/copy/move I/O so an already-present name stays a conflict, not an opaque fault.
pub(crate) fn map_conflict_io(path: PathBuf, source: io::Error) -> WorkspaceFileSystemError {
    if source.kind() == io::ErrorKind::AlreadyExists {
        WorkspaceFileSystemError::AlreadyExists { path }
    } else {
        WorkspaceFileSystemError::from_path_io(path, source)
    }
}

/// Splits a portable relative path into its existing parent and a single new name.
fn split_new_path(relative_path: &Path) -> Result<(PathBuf, String), WorkspaceFileSystemError> {
    let value =
        relative_path
            .to_str()
            .ok_or_else(|| WorkspaceFileSystemError::PathNotRelative {
                path: relative_path.to_path_buf(),
            })?;
    let parsed = ora_utils::path::PortableRelativePath::parse(value).map_err(|_| {
        WorkspaceFileSystemError::PathNotRelative {
            path: relative_path.to_path_buf(),
        }
    })?;
    let relative = parsed.as_str();
    if relative.is_empty() {
        return Err(WorkspaceFileSystemError::PathNotRelative {
            path: relative_path.to_path_buf(),
        });
    }
    match relative.rsplit_once('/') {
        None => Ok((PathBuf::new(), relative.to_string())),
        Some((parent, name)) if !parent.is_empty() && !name.is_empty() => {
            Ok((PathBuf::from(parent), name.to_string()))
        }
        Some(_) => Err(WorkspaceFileSystemError::PathNotRelative {
            path: relative_path.to_path_buf(),
        }),
    }
}

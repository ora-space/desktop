use std::path::{Path, PathBuf};

/// What a local Workspace location resolves to on disk at the moment it was probed.
pub(crate) enum LocalDirectory {
    /// The anchored path is an existing directory.
    Present(PathBuf),
    /// The anchored path is absent or not a directory; carries the path that was checked.
    Missing(PathBuf),
}

/// Anchors a local Workspace location to the runtime's stable path base and probes it now.
///
/// Cwd resolution, the `directory` reported by `listWorkspaces`, and project creation share this
/// rule, so a folder accepted at creation or reported present is exactly one that session start
/// and model discovery resolve. Branch listing and Effect materialization still read the stored
/// location with their own path handling (no anchoring; Effect also refuses a symlinked root), so
/// they can disagree for a relative or symlinked root.
///
/// Relative roots are stored against the directory from which `ORA_DATA_DIR` was
/// created. Joining them to a live process cwd would miss those directories
/// whenever the binary is started elsewhere — Desktop `tauri dev` starts in
/// `src-tauri`.
pub(crate) fn probe_local_directory(path: PathBuf, relative_path_base: &Path) -> LocalDirectory {
    let resolved = if path.is_absolute() {
        path
    } else {
        relative_path_base.join(path)
    };
    if resolved.is_dir() {
        LocalDirectory::Present(resolved)
    } else {
        LocalDirectory::Missing(resolved)
    }
}

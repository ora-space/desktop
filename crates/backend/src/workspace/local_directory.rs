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
/// This is the only rule for whether a local folder is usable. Project creation, the
/// `directory` reported by `listWorkspaces`, and every cwd resolution go through it, so a folder
/// accepted at creation or reported present is exactly one that session start and model
/// discovery resolve.
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

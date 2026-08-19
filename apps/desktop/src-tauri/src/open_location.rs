use crate::error::CommandError;
use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::{OpenLocationFailedParams, OpenLocationTarget, PublicError};
use serde::Deserialize;
#[cfg(any(test, windows, target_os = "macos"))]
use std::path::Path;
use tracing::Instrument;

/// Names the host application a location can be handed off to.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationTarget {
    Explorer,
    Terminal,
    VsCode,
}

/// Carries the target application and the absolute path it should open.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenLocationRequest {
    pub target: LocationTarget,
    pub path: String,
}

/// How the host file manager should present a path.
///
/// Directories open as folder windows. Files (and missing paths) must be
/// *revealed* so Windows Explorer / macOS Finder owns the window. Passing a
/// file to `explorer.exe` or `open` without a reveal switch launches the
/// default file association, which on developer machines is often Cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(any(test, windows, target_os = "macos"))]
pub(crate) enum ExplorerInvocation {
    OpenDirectory { path: String },
    RevealItem { path: String },
}

/// Converts Git-style slashes to the separator the host file manager expects.
#[cfg(any(test, windows, target_os = "macos"))]
fn native_location_path(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

/// Chooses reveal vs open-folder for the Explorer location target.
///
/// Only existing directories open as folder windows. Everything else —
/// regular files and missing paths — is revealed so a chat file link cannot
/// launch Cursor (or another default editor) in place of the system file manager.
#[cfg(any(test, windows, target_os = "macos"))]
pub(crate) fn explorer_invocation(path: &str) -> ExplorerInvocation {
    let path = native_location_path(path);
    if Path::new(&path).is_dir() {
        ExplorerInvocation::OpenDirectory { path }
    } else {
        ExplorerInvocation::RevealItem { path }
    }
}

/// Opens one absolute path in the file manager, a terminal, or VS Code on the host OS.
#[tauri::command]
pub async fn open_location(request: OpenLocationRequest) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start("open_location", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    let blocking_span = request_span.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        blocking_span.in_scope(|| open_location_blocking(request.target, &request.path))
    })
    .await
    {
        Ok(result) => result,
        Err(source) => Err(BackendError::internal(
            "Desktop command execution failed",
            source,
        )),
    };
    async move {
        match result {
            Ok(()) => {
                lifecycle.complete_success();
                Ok(())
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

/// Reports a location handoff that the host OS refused or could not launch.
fn open_location_error(
    target: LocationTarget,
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ora_backend::ErrorClassification::Internal,
        PublicError::OpenLocationFailed(OpenLocationFailedParams {
            target: match target {
                LocationTarget::Explorer => OpenLocationTarget::Explorer,
                LocationTarget::Terminal => OpenLocationTarget::Terminal,
                LocationTarget::VsCode => OpenLocationTarget::Vscode,
            },
        }),
        "failed to open the requested location",
        source,
    )
}

/// Launches the host handler for one location, branching per OS since only desktop hosts call this.
#[cfg(target_os = "windows")]
fn open_location_blocking(target: LocationTarget, path: &str) -> Result<(), BackendError> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    // CREATE_NO_WINDOW: keep the `cmd` shim that resolves `code.cmd` from flashing a console.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Git reports worktree paths with forward slashes; explorer.exe only navigates
    // backslash paths and silently falls back to a parent otherwise. Normalize once -
    // `wt`, PowerShell, and `code` all accept backslashes too.
    let normalized = native_location_path(path);
    let path = normalized.as_str();

    match target {
        // explorer.exe returns a non-zero exit code even on success, so a clean spawn is the only
        // signal worth trusting here.
        LocationTarget::Explorer => {
            spawn_windows_explorer(path).map_err(|source| open_location_error(target, source))
        }
        // `code` ships as `code.cmd`, which CreateProcess will not resolve directly; route it
        // through `cmd` and wait so a missing install surfaces as a failure the UI can report.
        LocationTarget::VsCode => {
            let status = Command::new("cmd")
                .args(["/C", "code", path])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|source| open_location_error(target, source))?;
            if status.success() {
                Ok(())
            } else {
                Err(open_location_error(
                    target,
                    std::io::Error::other(format!("VS Code exited with {status}")),
                ))
            }
        }
        // Prefer Windows Terminal; fall back to a PowerShell window opened in the target directory.
        LocationTarget::Terminal => {
            if Command::new("wt").args(["-d", path]).spawn().is_ok() {
                return Ok(());
            }
            Command::new("cmd")
                .args(["/C", "start", "", "/D", path, "powershell", "-NoExit"])
                .spawn()
                .map(|_| ())
                .map_err(|source| open_location_error(target, source))
        }
    }
}

/// Spawns Explorer for a folder or reveals a file without using the default association.
#[cfg(target_os = "windows")]
fn spawn_windows_explorer(path: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let mut command = Command::new("explorer");
    match explorer_invocation(path) {
        ExplorerInvocation::OpenDirectory { path } => {
            command.raw_arg(path);
        }
        ExplorerInvocation::RevealItem { path } => {
            // `/select,` is one switch; the path is a separate unquoted argument so
            // explorer.exe reveals the item instead of ShellExecute-opening it.
            command.raw_arg("/select,");
            command.raw_arg(path);
        }
    }
    command.spawn().map(|_| ())
}

/// Launches the host handler for one location through macOS `open`, which fails loudly when absent.
#[cfg(target_os = "macos")]
fn open_location_blocking(target: LocationTarget, path: &str) -> Result<(), BackendError> {
    use std::process::Command;

    let mut command = Command::new("open");
    match target {
        LocationTarget::Explorer => match explorer_invocation(path) {
            ExplorerInvocation::OpenDirectory { path } => {
                command.arg(path);
            }
            ExplorerInvocation::RevealItem { path } => {
                command.arg("-R").arg(path);
            }
        },
        LocationTarget::Terminal => {
            command.args(["-a", "Terminal", path]);
        }
        LocationTarget::VsCode => {
            command.args(["-a", "Visual Studio Code", path]);
        }
    }
    let status = command
        .status()
        .map_err(|source| open_location_error(target, source))?;
    if status.success() {
        Ok(())
    } else {
        Err(open_location_error(
            target,
            std::io::Error::other(format!("open command exited with {status}")),
        ))
    }
}

/// Rejects location handoffs on hosts that never run the desktop shell (only Web runs on Linux).
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_location_blocking(target: LocationTarget, _path: &str) -> Result<(), BackendError> {
    Err(open_location_error(
        target,
        std::io::Error::other("opening locations is unsupported on this platform"),
    ))
}

#[cfg(test)]
mod tests {
    use super::{ExplorerInvocation, explorer_invocation};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;

    /// Reveals a regular file so the default editor is not launched in its place.
    #[test]
    fn reveals_a_regular_file_instead_of_opening_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.rs");
        fs::write(&file, "fn main() {}\n").expect("write file");

        match explorer_invocation(&file.to_string_lossy()) {
            ExplorerInvocation::RevealItem { path } => {
                assert_eq!(Path::new(&path), file.as_path());
            }
            other => panic!("expected reveal, got {other:?}"),
        }
    }

    /// Git-style forward slashes still reveal a file after host-separator normalization.
    #[test]
    fn reveals_a_file_when_the_path_uses_forward_slashes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("lib.rs");
        fs::write(&file, "pub fn x() {}\n").expect("write file");
        let slashy = file.to_string_lossy().replace('\\', "/");

        match explorer_invocation(&slashy) {
            ExplorerInvocation::RevealItem { path } => {
                assert_eq!(Path::new(&path), file.as_path());
            }
            other => panic!("expected reveal, got {other:?}"),
        }
    }

    /// Workspace location actions still open a directory as a folder window.
    #[test]
    fn opens_a_directory_as_a_folder() {
        let dir = tempfile::tempdir().expect("temp dir");

        match explorer_invocation(&dir.path().to_string_lossy()) {
            ExplorerInvocation::OpenDirectory { path } => {
                assert_eq!(Path::new(&path), dir.path());
            }
            other => panic!("expected open directory, got {other:?}"),
        }
    }

    /// A manually deleted file is still revealed, not handed to Cursor as an open-file.
    #[test]
    fn reveals_a_missing_path_so_the_default_app_is_not_launched() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("deleted.rs");

        match explorer_invocation(&missing.to_string_lossy()) {
            ExplorerInvocation::RevealItem { path } => {
                assert_eq!(Path::new(&path), missing.as_path());
            }
            other => panic!("expected reveal, got {other:?}"),
        }
    }
}

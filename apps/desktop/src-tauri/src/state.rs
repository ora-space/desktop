use crate::surface::DesktopSurfaceService;
use crate::workspace_files::WorkspaceFileApi;
use ora_backend::{Backend, BackendPreferredLogLevelStore};
use ora_runtime_settings::RuntimeLogLevelManager;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub type DesktopRuntimeLogLevelManager =
    RuntimeLogLevelManager<ora_logging::LogLevelControl, BackendPreferredLogLevelStore>;

/// Stores every executable shipped with the Desktop application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledBinaryPaths {
    ripgrep: PathBuf,
    deno: PathBuf,
}

impl BundledBinaryPaths {
    /// Resolves the command paths used by the current Desktop build.
    pub fn resolve() -> Result<Self, BinaryResolutionError> {
        Ok(Self {
            ripgrep: resolve_binary("rg")?,
            deno: resolve_binary("deno")?,
        })
    }

    /// Returns the executable used by ora-fs and the shared backend for workspace search.
    pub fn ripgrep_path(&self) -> &PathBuf {
        &self.ripgrep
    }

    /// Returns the executable reserved for Rust-owned Deno integrations.
    pub fn deno_path(&self) -> &PathBuf {
        &self.deno
    }
}

/// Reports why a required shipped executable could not be resolved.
#[derive(Debug, Error)]
pub enum BinaryResolutionError {
    #[cfg(not(debug_assertions))]
    #[error("failed to resolve the Desktop executable directory")]
    CurrentExecutable(#[source] std::io::Error),
    #[cfg(not(debug_assertions))]
    #[error("required bundled binary {name} was not found at {path:?}")]
    Missing { name: &'static str, path: PathBuf },
}

/// Resolves one executable from PATH for debug builds and from the packaged app for releases.
#[cfg(debug_assertions)]
fn resolve_binary(executable_name: &'static str) -> Result<PathBuf, BinaryResolutionError> {
    // Development and test processes must follow the developer or CI toolchain instead of
    // requiring a target-specific download in the repository's binaries directory.
    Ok(PathBuf::from(executable_name))
}

/// Resolves one external binary beside the Tauri process in release builds.
#[cfg(not(debug_assertions))]
fn resolve_binary(executable_name: &'static str) -> Result<PathBuf, BinaryResolutionError> {
    let path = executable_directory()?.join(platform_binary_name(executable_name));
    if path.is_file() {
        Ok(path)
    } else {
        Err(BinaryResolutionError::Missing {
            name: executable_name,
            path,
        })
    }
}

/// Locates the directory where Tauri places external binaries in a packaged release.
#[cfg(not(debug_assertions))]
fn executable_directory() -> Result<PathBuf, BinaryResolutionError> {
    let executable = std::env::current_exe().map_err(BinaryResolutionError::CurrentExecutable)?;
    executable.parent().map(PathBuf::from).ok_or_else(|| {
        BinaryResolutionError::CurrentExecutable(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "current executable has no parent directory",
        ))
    })
}

/// Adds the native executable suffix expected by the current target platform.
#[cfg(not(debug_assertions))]
fn platform_binary_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Holds the shared Backend and Desktop configuration store managed by Tauri.
#[derive(Clone)]
pub struct DesktopState {
    pub backend: Backend,
    pub runtime_log_level: DesktopRuntimeLogLevelManager,
    pub workspace_files: Arc<WorkspaceFileApi>,
    pub binary_paths: BundledBinaryPaths,
    pub stream_cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Plugin surface host: native webviews, download delivery, plugin process linkage.
    pub surfaces: Arc<DesktopSurfaceService>,
}

impl DesktopState {
    /// Runs one host download action chosen by the trusted main webview for a webview-plugin
    /// download, then settles the tracked download.
    ///
    /// `import_skill` hands the landed archive to the existing two-phase skill import and returns
    /// its session id so the frontend can open the import preview; `save_as` copies the artifact
    /// to the destination the host save dialog chose. The staged host path never leaves the host.
    pub async fn resolve_surface_download(
        &self,
        download_id: u64,
        action: &str,
        destination: Option<String>,
    ) -> Result<crate::surface::commands::ResolveDownloadOutcome, crate::error::CommandError> {
        use crate::error::CommandError;
        use crate::surface::commands::ResolveDownloadOutcome;
        use crate::surface::download_actions::DownloadActionHost;
        use ora_backend::BackendError;
        use ora_backend::ErrorClassification;
        use ora_contracts::PublicError;
        use ora_plugin_manifest::DownloadAction;

        let parsed = action.parse::<DownloadAction>().map_err(|_| {
            CommandError::from_backend(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(ora_contracts::EmptyErrorParams {}),
                "unknown download action",
            ))
        })?;
        let staged = self
            .surfaces
            .take_download_for_action(download_id, parsed)
            .map_err(|_| {
                CommandError::from_backend(BackendError::new(
                    ErrorClassification::InvalidRequest,
                    PublicError::InvalidRequest(ora_contracts::EmptyErrorParams {}),
                    "download is not awaiting this action",
                ))
            })?;
        let outcome = match parsed {
            DownloadAction::ImportSkill => {
                // Same execution path as the automatic disposition (`DownloadActionHost`), so
                // prompt and auto can never drift apart in how an import is prepared.
                let response = DownloadActionHost::prepare_skill_import(
                    &self.backend,
                    &staged.path,
                    &staged.file_name,
                );
                match response {
                    Ok(session_id) => ResolveDownloadOutcome {
                        action: parsed.as_str().to_owned(),
                        import_session_id: Some(session_id),
                    },
                    Err(error) => {
                        self.surfaces
                            .settle_download(download_id, Some(error.to_string()));
                        return Err(CommandError::from_backend(error));
                    }
                }
            }
            DownloadAction::SaveAs => {
                let Some(destination) = destination else {
                    self.surfaces
                        .settle_download(download_id, Some("missing destination".to_owned()));
                    return Err(CommandError::from_backend(BackendError::new(
                        ErrorClassification::InvalidRequest,
                        PublicError::InvalidRequest(ora_contracts::EmptyErrorParams {}),
                        "save_as requires a destination path",
                    )));
                };
                if let Err(error) = std::fs::copy(&staged.path, &destination) {
                    self.surfaces
                        .settle_download(download_id, Some(error.to_string()));
                    return Err(CommandError::from_backend(BackendError::new(
                        ErrorClassification::Internal,
                        PublicError::InternalError(ora_contracts::EmptyErrorParams {}),
                        "failed to save the downloaded file",
                    )));
                }
                ResolveDownloadOutcome {
                    action: parsed.as_str().to_owned(),
                    import_session_id: None,
                }
            }
        };
        self.surfaces.settle_download(download_id, None);
        Ok(outcome)
    }
}

/// Retains process-scoped writer guards for the full Tauri application lifetime.
pub struct DesktopRuntimeGuard {
    pub _logging: ora_logging::LoggingGuard,
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{BundledBinaryPaths, PathBuf};
    use pretty_assertions::assert_eq;

    /// Verifies debug builds leave executable discovery to the inherited PATH.
    #[test]
    fn debug_builds_use_path_commands() {
        assert_eq!(
            BundledBinaryPaths::resolve().unwrap(),
            BundledBinaryPaths {
                ripgrep: PathBuf::from("rg"),
                deno: PathBuf::from("deno"),
            }
        );
    }
}

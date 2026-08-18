use crate::config::DesktopConfigStore;
use crate::workspace_files::WorkspaceFileApi;
use ora_backend::Backend;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// Stores every executable shipped with the Desktop application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledBinaryPaths {
    ripgrep: PathBuf,
    deno: PathBuf,
}

impl BundledBinaryPaths {
    /// Resolves all required binaries from the Tauri executable folder.
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
    #[error("failed to resolve the Desktop executable directory")]
    CurrentExecutable(#[source] std::io::Error),
    #[error("required bundled binary {name} was not found at {path:?}")]
    Missing { name: &'static str, path: PathBuf },
}

/// Resolves one external binary beside the Tauri process in development and release builds.
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

/// Locates the directory where Tauri dev and release place external binaries.
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
    pub config: DesktopConfigStore,
    pub workspace_files: Arc<WorkspaceFileApi>,
    pub binary_paths: BundledBinaryPaths,
    /// The Tauri application data directory, owner of the dashboard locator files.
    pub app_data_directory: PathBuf,
    pub stream_cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

/// Retains process-scoped writer guards for the full Tauri application lifetime.
pub struct DesktopRuntimeGuard {
    pub _logging: ora_logging::LoggingGuard,
}

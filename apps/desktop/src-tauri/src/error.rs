use crate::config::DesktopConfigError;
use ora_backend::{BackendBootstrapError, BackendError};
use serde::Serialize;
use thiserror::Error;

/// Reports failures that prevent the Desktop runtime from constructing its managed state.
#[derive(Debug, Error)]
pub enum DesktopBootstrapError {
    #[error("failed to resolve the system application data directory")]
    AppDataDirectory(#[source] tauri::Error),
    #[error(transparent)]
    Config(#[from] DesktopConfigError),
    #[error(transparent)]
    Logging(#[from] ora_logging::LoggingInitError),
    #[error(transparent)]
    Backend(#[from] BackendBootstrapError),
}

/// Carries the stable structured error payload returned by Tauri commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    code: &'static str,
    message: String,
}

impl CommandError {
    /// Creates a command failure from stable public fields.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Reports a blocking task join failure without exposing runtime internals.
    pub fn execution() -> Self {
        Self::new(
            "command_execution_error",
            "Desktop command execution failed",
        )
    }
}

impl From<BackendError> for CommandError {
    /// Preserves the shared backend error contract across the Tauri IPC boundary.
    fn from(error: BackendError) -> Self {
        Self::new(error.code(), error.message())
    }
}

impl From<DesktopConfigError> for CommandError {
    /// Maps Desktop-only configuration failures into stable frontend-visible codes.
    fn from(error: DesktopConfigError) -> Self {
        match error {
            DesktopConfigError::WorktreeRootNotAbsolute { .. } => Self::new(
                "worktree_root_not_absolute",
                "worktree root must be an absolute path",
            ),
            DesktopConfigError::WorktreeRootNotDirectory { .. } => Self::new(
                "worktree_root_not_directory",
                "worktree root must be an existing directory",
            ),
            DesktopConfigError::DashboardHostEmpty => Self::new(
                "dashboard_host_empty",
                "dashboard host must be a non-empty loopback address",
            ),
            DesktopConfigError::DashboardHostNotLoopback => Self::new(
                "dashboard_host_not_loopback",
                "dashboard host must be a loopback address (127.0.0.1, ::1, or localhost)",
            ),
            DesktopConfigError::DashboardPortZero => {
                Self::new("dashboard_port_zero", "dashboard port must be non-zero")
            }
            DesktopConfigError::Persist { .. } => Self::new(
                "desktop_config_persist_error",
                "failed to save Desktop configuration",
            ),
            DesktopConfigError::StateUnavailable => Self::new(
                "desktop_config_state_error",
                "Desktop configuration is unavailable",
            ),
            DesktopConfigError::DirectoryCreate { .. }
            | DesktopConfigError::Read { .. }
            | DesktopConfigError::Decode { .. }
            | DesktopConfigError::UnsupportedVersion { .. } => Self::new(
                "desktop_config_error",
                "Desktop configuration is unavailable",
            ),
        }
    }
}

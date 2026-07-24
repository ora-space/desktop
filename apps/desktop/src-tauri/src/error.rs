use crate::config::DesktopConfigError;
use ora_backend::{
    BackendBootstrapError, BackendError, ErrorClassification, RequestLifecycle,
    UuidRequestIdGenerator,
};
use ora_contracts::{ContractError, EmptyErrorParams, PublicError};
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

/// Serializes the transport-neutral contract directly across the Tauri command seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommandError(ContractError);

impl CommandError {
    /// Reports an adapter execution failure without exposing join or runtime internals.
    pub fn execution() -> Self {
        Self::from_backend(BackendError::new(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "Desktop command execution failed",
        ))
    }

    /// Completes one Tauri request and projects its typed public payload.
    pub fn from_backend(error: BackendError) -> Self {
        let lifecycle = RequestLifecycle::start("tauri_command", &UuidRequestIdGenerator);
        Self::from_backend_with_lifecycle(error, &lifecycle)
    }

    /// Projects a failure through a lifecycle created at the command entry seam.
    pub fn from_backend_with_lifecycle(error: BackendError, lifecycle: &RequestLifecycle) -> Self {
        lifecycle.complete_failure(&error);
        Self(error.contract_error(lifecycle.request_id()))
    }
}

impl From<BackendError> for CommandError {
    fn from(error: BackendError) -> Self {
        Self::from_backend(error)
    }
}

impl From<DesktopConfigError> for CommandError {
    fn from(error: DesktopConfigError) -> Self {
        Self::from_backend(desktop_config_backend_error(error))
    }
}

pub(crate) fn desktop_config_backend_error(error: DesktopConfigError) -> BackendError {
    let (classification, public_error, context) = match &error {
        DesktopConfigError::WorktreeRootNotAbsolute { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::WorktreeRootNotAbsolute(EmptyErrorParams {}),
            "worktree root must be an absolute path",
        ),
        DesktopConfigError::WorktreeRootNotDirectory { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::WorktreeRootNotDirectory(EmptyErrorParams {}),
            "worktree root must be an existing directory",
        ),
        // Dashboard endpoint validation failures are user-facing invalid request errors;
        // there is no dedicated PublicError variant so they are folded into the generic
        // InvalidRequest bucket with a context string that preserves the specifics.
        DesktopConfigError::DashboardHostEmpty
        | DesktopConfigError::DashboardHostNotLoopback
        | DesktopConfigError::DashboardPortZero => (
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "dashboard endpoint must be a non-empty loopback host with a non-zero port",
        ),
        DesktopConfigError::Persist { .. }
        | DesktopConfigError::StateUnavailable
        | DesktopConfigError::DirectoryCreate { .. }
        | DesktopConfigError::Read { .. }
        | DesktopConfigError::Decode { .. }
        | DesktopConfigError::UnsupportedVersion { .. } => (
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "Desktop configuration is unavailable",
        ),
    };
    BackendError::with_source(classification, public_error, context, error)
}

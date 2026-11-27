//! Tauri command seam for the Desktop updater.

use super::{DesktopUpdateStatus, UpdateError};
use crate::error::CommandError;
use crate::state::DesktopState;
use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};

/// Exposes the current status so a freshly mounted webview does not have to wait for an event.
#[tauri::command]
pub async fn get_desktop_update_status(
    state: tauri::State<'_, DesktopState>,
) -> Result<DesktopUpdateStatus, CommandError> {
    Ok(state.update.status())
}

/// Installs the verified cached update of the version the user confirmed.
#[tauri::command]
pub async fn install_desktop_update(
    version: String,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), CommandError> {
    // Boxed for the same IPC-stack reason every deep domain future is; see
    // `commands::run_async_backend`.
    Box::pin(state.update.install(&version))
        .await
        .map_err(install_command_error)
}

/// Completes a refused installation request and projects it for the webview.
///
/// A release replaced between the confirmation and the installation is an expected race: the host
/// has already published the replacement as ready, so it is a conflict the webview can ignore and
/// the request log records at WARN, not a fault of Ora's own.
pub(super) fn install_command_error(error: UpdateError) -> CommandError {
    let error = match error {
        UpdateError::ReleaseReplaced => BackendError::with_source(
            ErrorClassification::Conflict,
            PublicError::DesktopUpdateReleaseReplaced(EmptyErrorParams {}),
            "the confirmed Desktop update was replaced",
            error,
        ),
        error => BackendError::internal("failed to install Desktop update", error),
    };
    CommandError::from_backend(error)
}

/// Runs an update check on demand, outside the scheduled delayed and cron checks.
#[tauri::command]
pub async fn check_desktop_update(
    state: tauri::State<'_, DesktopState>,
) -> Result<(), CommandError> {
    Box::pin(state.update.check_and_download()).await;
    Ok(())
}

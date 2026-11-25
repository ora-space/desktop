//! Tauri command seam for the Desktop updater.

use super::DesktopUpdateStatus;
use crate::error::CommandError;
use crate::state::DesktopState;
use ora_backend::BackendError;

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
        .map_err(|error| {
            CommandError::from_backend(BackendError::internal(
                "failed to install Desktop update",
                error,
            ))
        })
}

/// Runs an update check on demand, outside the scheduled delayed and cron checks.
#[tauri::command]
pub async fn check_desktop_update(
    state: tauri::State<'_, DesktopState>,
) -> Result<(), CommandError> {
    Box::pin(state.update.check_and_download()).await;
    Ok(())
}

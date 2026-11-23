//! Desktop adapters for the Cloud connection; the use cases live in `crate::cloud_connection`.

use super::run_async_backend;

use ora_contracts::{
    CancelCloudLoginRequest, CloudConnection, GetCloudConnectionRequest, LogoutCloudRequest,
    LogoutCloudResponse, SetCloudServerRequest, SetCloudServerResponse, StartCloudLoginRequest,
};
use tauri::State;

use crate::error::CommandError;
use crate::state::DesktopState;

/// Returns the connection once startup has restored it.
#[tauri::command]
pub async fn get_cloud_connection(
    state: State<'_, DesktopState>,
    request: GetCloudConnectionRequest,
) -> Result<CloudConnection, CommandError> {
    let _ = request;
    let connection = state.cloud_connection.clone();
    run_async_backend(
        "get_cloud_connection",
        Box::pin(async move { Ok(connection.connection().await) }),
    )
    .await
}

/// Saves or forgets the Cloud server address.
#[tauri::command]
pub async fn set_cloud_server(
    state: State<'_, DesktopState>,
    request: SetCloudServerRequest,
) -> Result<SetCloudServerResponse, CommandError> {
    let connection = state.cloud_connection.clone();
    run_async_backend(
        "set_cloud_server",
        Box::pin(async move { connection.set_server(request.server).await }),
    )
    .await
}

/// Opens the system browser to sign in to the configured server.
#[tauri::command]
pub async fn start_cloud_login(
    state: State<'_, DesktopState>,
    request: StartCloudLoginRequest,
) -> Result<CloudConnection, CommandError> {
    let _ = request;
    let connection = state.cloud_connection.clone();
    run_async_backend(
        "start_cloud_login",
        Box::pin(async move { connection.start_login().await }),
    )
    .await
}

/// Abandons the sign-in in progress.
#[tauri::command]
pub async fn cancel_cloud_login(
    state: State<'_, DesktopState>,
    request: CancelCloudLoginRequest,
) -> Result<CloudConnection, CommandError> {
    let _ = request;
    let connection = state.cloud_connection.clone();
    run_async_backend(
        "cancel_cloud_login",
        Box::pin(async move { Ok(connection.cancel_login().await) }),
    )
    .await
}

/// Signs this Desktop out of the configured server.
#[tauri::command]
pub async fn logout_cloud(
    state: State<'_, DesktopState>,
    request: LogoutCloudRequest,
) -> Result<LogoutCloudResponse, CommandError> {
    let _ = request;
    let connection = state.cloud_connection.clone();
    run_async_backend(
        "logout_cloud",
        Box::pin(async move { Ok(connection.logout().await) }),
    )
    .await
}

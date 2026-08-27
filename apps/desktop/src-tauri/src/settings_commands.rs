//! Desktop adapters for developer preferences and process-wide runtime logging.

use std::future::Future;

use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::{
    DeveloperModeResponse, GetDeveloperModeRequest, GetProxySettingsRequest,
    GetProxySettingsResponse, GetRuntimeLogLevelRequest, ProxySettings, RuntimeLogLevel,
    RuntimeLogLevelStateResponse, SetDeveloperModeRequest, SetProxySettingsRequest,
    SetProxySettingsResponse, SetRuntimeLogLevelRequest,
};
use ora_runtime_settings::RuntimeLogLevelState;
use tauri::State;
use tracing::Instrument;

use crate::error::CommandError;
use crate::state::DesktopState;

/// Executes one asynchronous settings operation with the standard Tauri request lifecycle.
async fn run_async_command<Response, Call>(
    operation_name: &'static str,
    call: Call,
) -> Result<Response, CommandError>
where
    Call: Future<Output = Result<Response, BackendError>>,
{
    let lifecycle = RequestLifecycle::start(operation_name, &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());

    async move {
        match call.await {
            Ok(response) => {
                lifecycle.complete_success();
                Ok(response)
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

/// Returns the authoritative persisted developer-mode preference.
#[tauri::command]
pub async fn get_developer_mode(
    state: State<'_, DesktopState>,
    request: GetDeveloperModeRequest,
) -> Result<DeveloperModeResponse, CommandError> {
    let _ = request;
    let backend = state.backend.clone();
    run_async_command("get_developer_mode", async move {
        backend.developer_mode().await.map(developer_mode_response)
    })
    .await
}

/// Persists and returns the authoritative developer-mode preference.
#[tauri::command]
pub async fn set_developer_mode(
    state: State<'_, DesktopState>,
    request: SetDeveloperModeRequest,
) -> Result<DeveloperModeResponse, CommandError> {
    let backend = state.backend.clone();
    run_async_command("set_developer_mode", async move {
        backend
            .set_developer_mode(internal_developer_mode(request.enabled))
            .await
            .map(developer_mode_response)
    })
    .await
}

/// Returns the preferred and effective process-wide Desktop log levels.
#[tauri::command]
pub async fn get_runtime_log_level(
    state: State<'_, DesktopState>,
    request: GetRuntimeLogLevelRequest,
) -> Result<RuntimeLogLevelStateResponse, CommandError> {
    let _ = request;
    let manager = state.runtime_log_level.clone();
    run_async_command("get_runtime_log_level", async move {
        manager
            .state()
            .await
            .map(runtime_log_level_response)
            .map_err(|source| BackendError::internal("failed to read runtime log level", source))
    })
    .await
}

/// Replaces the live filter and persists the selected preferred log level.
#[tauri::command]
pub async fn set_runtime_log_level(
    state: State<'_, DesktopState>,
    request: SetRuntimeLogLevelRequest,
) -> Result<RuntimeLogLevelStateResponse, CommandError> {
    let manager = state.runtime_log_level.clone();
    let lifecycle = RequestLifecycle::start("set_runtime_log_level", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());

    async move {
        let result = manager
            .set_level(internal_log_level(request.level))
            .await
            .map(runtime_log_level_response)
            .map_err(|error| {
                if let Some(rollback_error) = error.rollback_error() {
                    let report = ora_logging::ErrorReport::from_error(rollback_error);
                    ora_logging::ora_error!(
                        operation = "set_runtime_log_level.rollback",
                        request_id = %lifecycle.request_id(),
                        outcome = "secondary_failure",
                        error.code = "internal_error",
                        error.message = report.message(),
                        error.chain = report.chain(),
                        error.chain_depth = report.chain_depth(),
                        "secondary cleanup failed"
                    );
                }
                BackendError::internal("failed to update runtime log level", error)
            });

        match result {
            Ok(response) => {
                lifecycle.complete_success();
                Ok(response)
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

/// Returns the optional configured network proxy.
#[tauri::command]
pub async fn get_proxy_settings(
    state: State<'_, DesktopState>,
    request: GetProxySettingsRequest,
) -> Result<GetProxySettingsResponse, CommandError> {
    let _ = request;
    let backend = state.backend.clone();
    run_async_command("get_proxy_settings", async move {
        backend
            .network_proxy_settings()
            .map(proxy_settings_response)
    })
    .await
}
/// Persists and returns the configured network proxy.
#[tauri::command]
pub async fn set_proxy_settings(
    state: State<'_, DesktopState>,
    request: SetProxySettingsRequest,
) -> Result<SetProxySettingsResponse, CommandError> {
    let backend = state.backend.clone();
    run_async_command("set_proxy_settings", async move {
        let settings = internal_network_proxy_settings(request.settings);
        backend
            .set_network_proxy_settings(settings)
            .map(set_proxy_settings_response)
    })
    .await
}
/// Converts optional persisted proxy settings into the Settings response shape.
fn proxy_settings_response(
    settings: Option<ora_application::NetworkProxySettings>,
) -> GetProxySettingsResponse {
    GetProxySettingsResponse {
        settings: settings.map(proxy_settings_contract),
    }
}
/// Converts saved proxy settings into the authoritative Settings response shape.
fn set_proxy_settings_response(
    settings: ora_application::NetworkProxySettings,
) -> SetProxySettingsResponse {
    SetProxySettingsResponse {
        settings: Some(proxy_settings_contract(settings)),
    }
}
/// Converts persisted proxy settings into one transport-neutral wire value.
fn proxy_settings_contract(settings: ora_application::NetworkProxySettings) -> ProxySettings {
    ProxySettings {
        host: settings.host,
        port: settings.port,
        username: settings.username,
        password: settings.password,
    }
}
/// Converts transport-neutral proxy settings into the persistence-backed application value.
fn internal_network_proxy_settings(
    settings: ProxySettings,
) -> ora_application::NetworkProxySettings {
    ora_application::NetworkProxySettings {
        host: settings.host,
        port: settings.port,
        username: settings.username,
        password: settings.password,
    }
}
/// Converts the application enum into the shared response contract.
fn developer_mode_response(mode: ora_application::DeveloperMode) -> DeveloperModeResponse {
    DeveloperModeResponse {
        enabled: mode.is_enabled(),
    }
}

/// Converts the transport boolean into an explicit application state.
fn internal_developer_mode(enabled: bool) -> ora_application::DeveloperMode {
    if enabled {
        ora_application::DeveloperMode::Enabled
    } else {
        ora_application::DeveloperMode::Disabled
    }
}

/// Converts manager state into the transport-neutral response contract.
fn runtime_log_level_response(state: RuntimeLogLevelState) -> RuntimeLogLevelStateResponse {
    RuntimeLogLevelStateResponse {
        configured_level: contract_log_level(state.configured_level),
        effective_level: contract_log_level(state.effective_level),
        startup_override: state.startup_override.map(contract_log_level),
    }
}

/// Converts an internal level into the closed wire vocabulary.
fn contract_log_level(level: ora_logging::LogLevel) -> RuntimeLogLevel {
    match level {
        ora_logging::LogLevel::Trace => RuntimeLogLevel::Trace,
        ora_logging::LogLevel::Debug => RuntimeLogLevel::Debug,
        ora_logging::LogLevel::Info => RuntimeLogLevel::Info,
        ora_logging::LogLevel::Warn => RuntimeLogLevel::Warn,
        ora_logging::LogLevel::Error => RuntimeLogLevel::Error,
    }
}

/// Converts a validated contract level into the shared logging vocabulary.
fn internal_log_level(level: RuntimeLogLevel) -> ora_logging::LogLevel {
    match level {
        RuntimeLogLevel::Trace => ora_logging::LogLevel::Trace,
        RuntimeLogLevel::Debug => ora_logging::LogLevel::Debug,
        RuntimeLogLevel::Info => ora_logging::LogLevel::Info,
        RuntimeLogLevel::Warn => ora_logging::LogLevel::Warn,
        RuntimeLogLevel::Error => ora_logging::LogLevel::Error,
    }
}

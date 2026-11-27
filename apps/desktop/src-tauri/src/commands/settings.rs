//! Desktop adapters for developer preferences and process-wide runtime logging.

use super::{run_async_backend, run_async_backend_with_request_id};

use ora_backend::BackendError;
use ora_contracts::{
    CheckProxySettingsRequest, CheckProxySettingsResponse, ClearProxySettingsRequest,
    ClearProxySettingsResponse, DeveloperModeResponse, GetDeveloperModeRequest,
    GetProxySettingsRequest, GetProxySettingsResponse, GetRuntimeLogLevelRequest, ProxySettings,
    RuntimeLogLevel, RuntimeLogLevelStateResponse, SetDeveloperModeRequest,
    SetProxySettingsRequest, SetProxySettingsResponse, SetRuntimeLogLevelRequest,
};
use ora_runtime_settings::{
    PreferredLogLevelStore, RuntimeLogLevelControl, RuntimeLogLevelManager, RuntimeLogLevelState,
};
use tauri::State;

use crate::error::CommandError;
use crate::state::DesktopState;

/// Reads the durable probe target without changing the proxy itself.
#[tauri::command]
pub async fn get_proxy_check_url(
    state: State<'_, DesktopState>,
    request: ora_contracts::GetProxyCheckUrlRequest,
) -> Result<ora_contracts::ProxyCheckUrlResponse, CommandError> {
    let _ = request;
    let settings = state.backend.settings().clone();
    run_async_backend(
        "get_proxy_check_url",
        Box::pin(async move {
            settings
                .network_proxy_check_url()
                .await
                .map(|url| ora_contracts::ProxyCheckUrlResponse { url })
        }),
    )
    .await
}

/// Saves only the confirmed probe target; request failures leave its previous value intact.
#[tauri::command]
pub async fn set_proxy_check_url(
    state: State<'_, DesktopState>,
    request: ora_contracts::SetProxyCheckUrlRequest,
) -> Result<ora_contracts::ProxyCheckUrlResponse, CommandError> {
    let settings = state.backend.settings().clone();
    run_async_backend(
        "set_proxy_check_url",
        Box::pin(async move {
            settings
                .set_network_proxy_check_url(request.url)
                .await
                .map(|url| ora_contracts::ProxyCheckUrlResponse { url })
        }),
    )
    .await
}

/// Returns the authoritative persisted developer-mode preference.
#[tauri::command]
pub async fn get_developer_mode(
    state: State<'_, DesktopState>,
    request: GetDeveloperModeRequest,
) -> Result<DeveloperModeResponse, CommandError> {
    let _ = request;
    let settings_handle = state.backend.settings().clone();
    run_async_backend(
        "get_developer_mode",
        Box::pin(async move {
            settings_handle
                .developer_mode()
                .await
                .map(developer_mode_response)
        }),
    )
    .await
}

/// Persists and returns the authoritative developer-mode preference.
#[tauri::command]
pub async fn set_developer_mode(
    state: State<'_, DesktopState>,
    request: SetDeveloperModeRequest,
) -> Result<DeveloperModeResponse, CommandError> {
    let settings_handle = state.backend.settings().clone();
    run_async_backend(
        "set_developer_mode",
        Box::pin(async move {
            settings_handle
                .set_developer_mode(internal_developer_mode(request.enabled))
                .await
                .map(developer_mode_response)
        }),
    )
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
    run_async_backend(
        "get_runtime_log_level",
        Box::pin(async move {
            manager
                .state()
                .await
                .map(runtime_log_level_response)
                .map_err(|source| {
                    BackendError::internal("failed to read runtime log level", source)
                })
        }),
    )
    .await
}

/// Replaces the live filter and persists the selected preferred log level.
#[tauri::command]
pub async fn set_runtime_log_level(
    state: State<'_, DesktopState>,
    request: SetRuntimeLogLevelRequest,
) -> Result<RuntimeLogLevelStateResponse, CommandError> {
    update_runtime_log_level(state.runtime_log_level.clone(), request).await
}

/// Keeps rollback policy in Settings while the executor owns request completion.
async fn update_runtime_log_level<C, S>(
    manager: RuntimeLogLevelManager<C, S>,
    request: SetRuntimeLogLevelRequest,
) -> Result<RuntimeLogLevelStateResponse, CommandError>
where
    C: RuntimeLogLevelControl,
    S: PreferredLogLevelStore,
{
    run_async_backend_with_request_id("set_runtime_log_level", move |request_id| {
        Box::pin(async move {
            manager
                .set_level(internal_log_level(request.level))
                .await
                .map(runtime_log_level_response)
                .map_err(|error| {
                    if let Some(rollback_error) = error.rollback_error() {
                        let report = ora_logging::ErrorReport::from_error(rollback_error);
                        ora_logging::ora_error!(
                            operation = "set_runtime_log_level.rollback",
                            request_id = %request_id,
                            outcome = "secondary_failure",
                            error.code = "internal_error",
                            error.message = report.message(),
                            error.chain = report.chain(),
                            error.chain_depth = report.chain_depth(),
                            "secondary cleanup failed"
                        );
                    }
                    BackendError::internal("failed to update runtime log level", error)
                })
        })
    })
    .await
}

/// Returns the optional configured network proxy.
#[tauri::command]
pub async fn get_proxy_settings(
    state: State<'_, DesktopState>,
    request: GetProxySettingsRequest,
) -> Result<GetProxySettingsResponse, CommandError> {
    let settings_handle = state.backend.settings().clone();
    run_async_backend(
        "get_proxy_settings",
        Box::pin(async move {
            if request.load_manual == Some(true) {
                let preferences = ora_application::NetworkProxyPreferences {
                    mode: ora_application::NetworkProxyMode::Manual,
                    manual: settings_handle.network_proxy_settings().await?,
                };
                if preferences.manual.is_some() {
                    ora_backend::Settings::network_proxy_policy_for(&preferences)?;
                }
                return Ok(GetProxySettingsResponse {
                    mode: None,
                    settings: preferences.manual.map(proxy_settings_contract),
                });
            }
            settings_handle
                .network_proxy_preferences()
                .await
                .map(proxy_settings_response)
        }),
    )
    .await
}
/// Persists and returns the configured network proxy.
#[tauri::command]
pub async fn set_proxy_settings(
    state: State<'_, DesktopState>,
    request: SetProxySettingsRequest,
) -> Result<SetProxySettingsResponse, CommandError> {
    let settings_handle = state.backend.settings().clone();
    run_async_backend(
        "set_proxy_settings",
        Box::pin(async move {
            let preferences = internal_network_proxy_preferences(request.mode, request.settings);
            settings_handle
                .set_network_proxy_preferences(preferences)
                .await
                .map(set_proxy_settings_response)
        }),
    )
    .await
}

/// Removes the configured network proxy.
#[tauri::command]
pub async fn clear_proxy_settings(
    state: State<'_, DesktopState>,
    request: ClearProxySettingsRequest,
) -> Result<ClearProxySettingsResponse, CommandError> {
    let _ = request;
    let settings_handle = state.backend.settings().clone();
    run_async_backend(
        "clear_proxy_settings",
        Box::pin(async move {
            settings_handle.clear_network_proxy_settings().await?;
            Ok(ClearProxySettingsResponse {
                settings: None,
                mode: Some(ora_contracts::ProxyMode::System),
            })
        }),
    )
    .await
}

/// Probes one URL through the supplied proxy configuration without persisting it.
#[tauri::command]
pub async fn check_proxy_settings(
    request: CheckProxySettingsRequest,
) -> Result<CheckProxySettingsResponse, CommandError> {
    run_async_backend(
        "check_proxy_settings",
        Box::pin(async move {
            let preferences = internal_network_proxy_preferences(request.mode, request.settings);
            Ok(crate::network_proxy::check(&preferences, &request.url).await)
        }),
    )
    .await
}
/// Converts optional persisted proxy settings into the Settings response shape.
fn proxy_settings_response(
    preferences: ora_application::NetworkProxyPreferences,
) -> GetProxySettingsResponse {
    GetProxySettingsResponse {
        settings: preferences.manual.map(proxy_settings_contract),
        mode: Some(proxy_mode_contract(preferences.mode)),
    }
}
/// Converts saved proxy settings into the authoritative Settings response shape.
fn set_proxy_settings_response(
    preferences: ora_application::NetworkProxyPreferences,
) -> SetProxySettingsResponse {
    SetProxySettingsResponse {
        settings: preferences.manual.map(proxy_settings_contract),
        mode: Some(proxy_mode_contract(preferences.mode)),
    }
}
/// Exposes the effective mode independently of the retained manual settings.
fn proxy_mode_contract(mode: ora_application::NetworkProxyMode) -> ora_contracts::ProxyMode {
    use ora_application::NetworkProxyMode as Internal;
    use ora_contracts::ProxyMode;
    match mode {
        Internal::System => ProxyMode::System,
        Internal::Manual => ProxyMode::Manual,
        Internal::Direct => ProxyMode::Direct,
    }
}

/// Converts a form's unsaved mode and optional manual input.
fn internal_network_proxy_preferences(
    mode: Option<ora_contracts::ProxyMode>,
    settings: Option<ProxySettings>,
) -> ora_application::NetworkProxyPreferences {
    use ora_application::NetworkProxyMode as Internal;
    use ora_contracts::ProxyMode;
    let mode = match mode.unwrap_or(ProxyMode::Manual) {
        ProxyMode::System => Internal::System,
        ProxyMode::Manual => Internal::Manual,
        ProxyMode::Direct => Internal::Direct,
    };
    ora_application::NetworkProxyPreferences {
        mode,
        manual: settings.map(internal_network_proxy_settings),
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

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;

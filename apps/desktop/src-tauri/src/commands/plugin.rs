//! Desktop plugin operations.

use super::run_async_backend;
use crate::{error::CommandError, state::DesktopState};
use ora_contracts::*;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tauri::{AppHandle, Emitter, State};

const PLUGIN_INSTALL_PROGRESS_EVENT: &str = "plugin-install-progress";
const PLUGIN_PROGRESS_EVENT_INTERVAL: u64 = 1024 * 1024;

/// Carries byte-level marketplace package progress to the plugin card that started the operation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallProgressEvent {
    plugin_id: String,
    downloaded: u64,
    total: Option<u64>,
}

/// Builds the shared throttled event callback used by marketplace installs and updates.
///
/// Emission is throttled to whole intervals so a fast transfer cannot flood the webview, while
/// the terminal snapshot always reaches the card that started the operation.
fn plugin_transfer_progress(
    app: AppHandle,
    plugin_id: String,
) -> ora_utils::http::ProgressCallback {
    let last_emitted = Arc::new(AtomicU64::new(0));
    Arc::new(move |progress: ora_utils::http::Progress| {
        let previous = last_emitted.load(Ordering::Relaxed);
        let complete = progress.total == Some(progress.bytes);
        if previous != 0
            && progress.bytes < previous.saturating_add(PLUGIN_PROGRESS_EVENT_INTERVAL)
            && !complete
        {
            return;
        }
        last_emitted.store(progress.bytes, Ordering::Relaxed);
        let event = PluginInstallProgressEvent {
            plugin_id: plugin_id.clone(),
            downloaded: progress.bytes,
            total: progress.total,
        };
        if let Err(error) = app.emit(PLUGIN_INSTALL_PROGRESS_EVENT, event) {
            ora_logging::ora_warn!(message = "Failed to emit plugin transfer progress", error = %error);
        }
    })
}

backend_command!(
    list_installed_plugins,
    ListInstalledPluginsRequest,
    ListInstalledPluginsResponse,
    plugins.list_installed,
    "Lists the cached installed-plugin lifecycle snapshot."
);
backend_command!(
    get_plugin_configuration,
    GetPluginConfigurationRequest,
    GetPluginConfigurationResponse,
    plugins.get_configuration,
    "Loads one typed Plugin Configuration editor snapshot."
);
backend_command!(
    save_plugin_configuration,
    SavePluginConfigurationRequest,
    SavePluginConfigurationResponse,
    plugins.save_configuration,
    "Persists one revision-checked Plugin Configuration replacement."
);
backend_command!(
    reset_plugin_configuration,
    ResetPluginConfigurationRequest,
    ResetPluginConfigurationResponse,
    plugins.reset_configuration,
    "Resets explicit overrides or recovers a damaged Plugin Configuration."
);
backend_command!(
    list_available_plugins,
    ListAvailablePluginsRequest,
    ListAvailablePluginsResponse,
    plugins.list_available,
    "Lists the cached marketplace registry index."
);
backend_command!(
    sync_available_plugins,
    SyncAvailablePluginsRequest,
    SyncAvailablePluginsResponse,
    plugins.sync_available,
    "Pulls the marketplace source and rebuilds the cached registry index."
);
backend_command!(
    read_plugin_readme,
    ReadPluginReadmeRequest,
    ReadPluginReadmeResponse,
    plugins.read_readme,
    "Reads one marketplace plugin's published README for its detail page."
);
backend_command!(
    list_pack_installations,
    ListPackInstallationsRequest,
    ListPackInstallationsResponse,
    plugins.list_pack_installations,
    "Lists every recorded pack installation with its reconciled member states."
);
backend_command!(
    pack_uninstall_plan,
    PackUninstallPlanRequest,
    PackUninstallPlanResponse,
    plugins.pack_uninstall_plan,
    "Computes the ownership-aware uninstall plan for one recorded pack."
);
backend_command!(
    list_hook_lifecycle_reports,
    ListHookLifecycleReportsRequest,
    ListHookLifecycleReportsResponse,
    plugins.list_hook_lifecycle_reports,
    "Lists this session's Hook lifecycle results, keyed by plugin."
);

backend_command!(
    list_marketplace_sources,
    ListMarketplaceSourcesRequest,
    ListMarketplaceSourcesResponse,
    plugins.list_sources,
    "Lists the configured marketplace source repositories."
);
backend_command!(
    add_marketplace_source,
    AddMarketplaceSourceRequest,
    AddMarketplaceSourceResponse,
    plugins.add_source,
    "Adds one marketplace source repository."
);
backend_command!(
    delete_marketplace_source,
    DeleteMarketplaceSourceRequest,
    DeleteMarketplaceSourceResponse,
    plugins.delete_source,
    "Removes one marketplace source repository."
);
backend_command!(
    update_marketplace_source,
    UpdateMarketplaceSourceRequest,
    UpdateMarketplaceSourceResponse,
    plugins.update_source,
    "Updates one marketplace source's URL, branch, proxy policy, or enabled state."
);
async_backend_command!(
    scan_plugins,
    ScanPluginsRequest,
    ScanPluginsResponse,
    plugins.scan,
    "Explicitly scans and reconciles installed plugins."
);
async_backend_command!(
    activate_plugin,
    ActivatePluginRequest,
    ActivatePluginResponse,
    plugins.activate,
    "Activates one installed plugin."
);
async_backend_command!(
    stop_plugin,
    StopPluginRequest,
    StopPluginResponse,
    plugins.stop,
    "Stops one plugin process."
);
async_backend_command!(
    uninstall_plugin,
    UninstallPluginRequest,
    UninstallPluginResponse,
    plugins.uninstall,
    "Stops and removes one installed plugin."
);
backend_command!(
    get_plugin_log_level,
    GetPluginLogLevelRequest,
    PluginLogLevelResponse,
    plugins.get_log_level,
    "Reads one plugin's host-owned log level."
);
async_backend_command!(
    set_plugin_log_level,
    SetPluginLogLevelRequest,
    PluginLogLevelResponse,
    plugins.set_log_level,
    "Persists and applies one plugin's host-owned log level."
);
/// Installs one marketplace plugin and emits throttled byte-level download progress.
#[tauri::command]
pub async fn install_plugin(
    state: State<'_, DesktopState>,
    app: AppHandle,
    request: InstallPluginRequest,
) -> Result<InstallPluginResponse, CommandError> {
    let progress = plugin_transfer_progress(app, request.plugin_id.clone());
    let plugins = state.backend.plugins();
    // The marketplace install chain is the deepest command the Desktop surface drives: it
    // monomorphizes into a future hundreds of KB in size, and constructing that future on the
    // IPC main thread is what killed release 0.2.0 with a stack overflow. Boxing keeps the
    // command future pointer-sized; see `run_async_backend`.
    run_async_backend(
        "install_plugin",
        Box::pin(plugins.install_with_progress(request, progress)),
    )
    .await
}

/// Updates one marketplace plugin and emits throttled byte-level download progress.
#[tauri::command]
pub async fn update_plugin(
    state: State<'_, DesktopState>,
    app: AppHandle,
    request: UpdatePluginRequest,
) -> Result<UpdatePluginResponse, CommandError> {
    let progress = plugin_transfer_progress(app, request.plugin_id.clone());
    let plugins = state.backend.plugins();
    // Same transfer chain and same IPC-stack constraint as `install_plugin` above.
    run_async_backend(
        "update_plugin",
        Box::pin(plugins.update_with_progress(request, progress)),
    )
    .await
}
async_backend_command!(
    import_plugin,
    ImportPluginRequest,
    ImportPluginResponse,
    plugins.import,
    "Imports one local .orax release archive; the installed plugin is immediately available."
);
backend_command!(
    retry_workflow_import,
    RetryWorkflowImportRequest,
    RetryWorkflowImportResponse,
    plugins.retry_workflow_import,
    "Retries the workflow document import of one installed plugin without reinstalling it."
);
async_backend_command!(
    initialize_hook,
    InitializeHookRequest,
    InitializeHookResponse,
    plugins.initialize_hook,
    "Runs one installed Hook package's declared `init` command."
);
backend_command!(
    list_mcp_health,
    ListMcpHealthRequest,
    ListMcpHealthResponse,
    plugins.list_mcp_health,
    "Lists Host MCP health for currently eligible installed members."
);
async_backend_command!(
    probe_mcp_health,
    ProbeMcpHealthRequest,
    ProbeMcpHealthResponse,
    plugins.probe_mcp_health,
    "Awaits one Host MCP health probe for a currently eligible member."
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{IPC_COMMAND_FUTURE_BUDGET, run_async_backend};
    use ora_backend::{Backend, BackendPaths};
    use ora_contracts::{InstallPluginRequest, UpdatePluginRequest};
    use ora_utils::http::ProgressCallback;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Opens a throwaway Backend rooted in one isolated data directory.
    ///
    /// The futures below are never polled, so no marketplace source is contacted and the missing
    /// `deno` executable is never launched; only their construction matters here.
    fn test_backend(root: &std::path::Path) -> Backend {
        ora_logging::initialize_test_clock();
        Backend::open(BackendPaths {
            app_data_directory: root.to_path_buf(),
            home_directory: root.to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            relative_path_base: root.to_path_buf(),
            timezone: chrono_tz::UTC,
        })
        .expect("open backend")
    }

    /// Guards the marketplace transfer commands against regrowing the future the IPC thread
    /// materializes.
    ///
    /// Tauri constructs every async command's future on the main thread inside the WebView2 IPC
    /// callback, whose ~1 MB stack the webview/tauri frames already occupy for several hundred
    /// KB. Release 0.2.0 crashed the process with a main-thread stack overflow (WER
    /// `0xc00000fd`) the moment a marketplace install was clicked, because the install chain
    /// monomorphizes into a single ~650 KB state machine and constructing it on that stack
    /// overflowed before the async runtime ever polled it (see `IPC_COMMAND_FUTURE_BUDGET`).
    ///
    /// This measures the executor wrapper the command holds across its await — the part that
    /// dominates the command future — against the budget. The deep transfer future itself stays
    /// huge by design; it is only ever constructed on the runtime thread that first polls the
    /// wrapper, which is exactly what the boxing at the `run_async_backend` seam guarantees.
    #[test]
    fn marketplace_transfer_commands_stay_within_the_ipc_future_budget() {
        ora_logging::with_trace_logging(|| {
            let directory = TempDir::new().expect("temp directory");
            let backend = test_backend(directory.path());
            let plugins = backend.plugins();
            let progress: ProgressCallback = Arc::new(|_| {});
            let install = run_async_backend(
                "install_plugin",
                Box::pin(plugins.install_with_progress(
                    InstallPluginRequest {
                        plugin_id: "official/absent".to_owned(),
                        hook_execution_acknowledged: false,
                    },
                    progress.clone(),
                )),
            );
            let update = run_async_backend(
                "update_plugin",
                Box::pin(plugins.update_with_progress(
                    UpdatePluginRequest {
                        plugin_id: "official/absent".to_owned(),
                        hook_execution_acknowledged: false,
                    },
                    progress,
                )),
            );

            let sizes = [
                ("install_plugin", std::mem::size_of_val(&install)),
                ("update_plugin", std::mem::size_of_val(&update)),
            ];
            for (name, size) in sizes {
                assert!(
                    size <= IPC_COMMAND_FUTURE_BUDGET,
                    "{name} future is {size} bytes, above the {IPC_COMMAND_FUTURE_BUDGET}-byte IPC \
                     budget: deep domain futures must stay boxed at the run_async_backend seam",
                );
            }
        });
    }
}

use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, ListInstalledPluginsRequest,
    ListInstalledPluginsResponse, ScanPluginsRequest, ScanPluginsResponse, StopPluginRequest,
    StopPluginResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_db::{RepositoryPool, SqlitePluginStateRepository};
use ora_logging::ora_debug;
use ora_plugin_lifecycle::{
    DenoPluginRuntimeLauncher, InboundNotification, PluginLifecycle, PluginLifecycleConfig,
    PluginLifecycleError, PluginNotificationSink, PluginRuntimeTimeouts,
};
use std::path::PathBuf;

/// The concrete lifecycle composition the backend runs.
pub(crate) type BackendPluginLifecycle = PluginLifecycle<
    SqlitePluginStateRepository,
    SystemClock,
    DenoPluginRuntimeLauncher,
    AppEventPublisher,
    LoggingNotificationSink,
>;

/// Logs plugin-originated notifications until a surface broker consumes them.
///
/// The first ui plugins declare no `emits`, so nothing arrives here in practice; logging keeps
/// the stream observable without committing to a routing design before one is needed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LoggingNotificationSink;

impl PluginNotificationSink for LoggingNotificationSink {
    /// Records the notification at debug level and drops it.
    fn on_notification(&self, notification: InboundNotification) {
        ora_debug!(
            message = "plugin notification received",
            plugin_id = %notification.plugin_id,
            generation = notification.generation.0,
            method = %notification.method,
        );
    }
}

/// Groups plugin discovery and lifecycle operations behind the backend's plugin interface.
pub(crate) struct PluginApi {
    lifecycle: BackendPluginLifecycle,
}

impl PluginApi {
    /// Opens plugin lifecycle state with the concrete backend adapters.
    pub(crate) fn open(
        pool: RepositoryPool,
        data_directory: PathBuf,
        deno_path: PathBuf,
        clock: SystemClock,
        publisher: AppEventPublisher,
    ) -> Result<Self, PluginLifecycleError> {
        let lifecycle = PluginLifecycle::open(
            PluginLifecycleConfig {
                data_directory,
                deno_path,
            },
            SqlitePluginStateRepository::new(pool),
            clock,
            DenoPluginRuntimeLauncher::new(PluginRuntimeTimeouts::default()),
            publisher,
            LoggingNotificationSink,
        )?;

        Ok(Self { lifecycle })
    }

    /// Exposes the lifecycle to the gateway that serves desktop surfaces.
    pub(crate) fn lifecycle(&self) -> &BackendPluginLifecycle {
        &self.lifecycle
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub(crate) fn list(
        &self,
        _request: ListInstalledPluginsRequest,
    ) -> ListInstalledPluginsResponse {
        self.lifecycle.list_installed_plugins()
    }

    /// Rescans packages and reconciles durable and runtime state.
    pub(crate) async fn scan(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, PluginLifecycleError> {
        self.lifecycle.scan_plugins(request).await
    }

    /// Persists plugin eligibility without starting its process.
    pub(crate) async fn enable(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, PluginLifecycleError> {
        self.lifecycle.enable_plugin(request).await
    }

    /// Stops a plugin when necessary before persisting ineligibility.
    pub(crate) async fn disable(
        &self,
        request: DisablePluginRequest,
    ) -> Result<DisablePluginResponse, PluginLifecycleError> {
        self.lifecycle.disable_plugin(request).await
    }

    /// Starts one enabled plugin and returns its immediate starting state.
    pub(crate) async fn activate(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, PluginLifecycleError> {
        self.lifecycle.activate_plugin(request).await
    }

    /// Stops one plugin process without changing durable eligibility.
    pub(crate) async fn stop(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, PluginLifecycleError> {
        self.lifecycle.stop_plugin(request).await
    }

    /// Stops and removes one plugin package plus its durable state.
    pub(crate) async fn uninstall(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, PluginLifecycleError> {
        self.lifecycle.uninstall_plugin(request).await
    }
}

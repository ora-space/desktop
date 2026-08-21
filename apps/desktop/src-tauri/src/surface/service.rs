//! `SurfaceService`: the composition root that turns frontend intents into registry transitions
//! and executes the resulting effects against Tauri webviews.

use crate::surface::capabilities::SurfaceCapabilities;
use crate::surface::downloads::DownloadDispatcher;
use crate::surface::error::SurfaceError;
use crate::surface::gateway::SurfacePluginGateway;
use crate::surface::idle::IdleTimers;
use crate::surface::push::PushSequencer;
use crate::surface::windowed::WindowedAdapter;
use ora_domain::PluginId;
use ora_logging::ora_warn;
use ora_plugin_lifecycle::SurfaceCloser;
use ora_surface::{
    DownloadClock, LocalDownloadClock, MountTarget, OpenError, SurfaceCommand, SurfaceDefinition,
    SurfaceInstanceId, SurfaceRecord, SurfaceRegistry, SurfaceState,
};
use std::future::Future;
use std::sync::{Arc, Weak};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime};

/// Owns every live surface of the process.
///
/// `C` is the clock stamping downloads; it is injectable so tests can run without the
/// process-wide logging clock.
pub struct SurfaceService<
    G: SurfacePluginGateway,
    R: Runtime,
    C: DownloadClock = LocalDownloadClock,
> {
    pub(super) app: AppHandle<R>,
    pub(super) gateway: G,
    pub(super) registry: Arc<SurfaceRegistry>,
    pub(super) downloads: Arc<DownloadDispatcher<G, R, C>>,
    pub(super) windowed: WindowedAdapter<R>,
    #[cfg(feature = "embedded-surfaces")]
    pub(super) embedded: crate::surface::embedded::EmbeddedAdapter<R>,
    pub(super) idle: IdleTimers,
    pub(super) pushes: PushSequencer,
    pub(super) capabilities: SurfaceCapabilities,
    /// Handed to window callbacks so a closed window can reach the service without a cycle.
    pub(super) weak: Weak<Self>,
}

impl<G: SurfacePluginGateway, R: Runtime> SurfaceService<G, R> {
    /// Builds the production service stamping downloads with the process-local clock.
    pub fn new(app: AppHandle<R>, gateway: G) -> Arc<Self> {
        Self::with_clock(app, gateway, LocalDownloadClock)
    }
}

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock + Send + Sync + 'static>
    SurfaceService<G, R, C>
{
    /// Builds the service with an explicit download clock and probes embedded support once.
    pub fn with_clock(app: AppHandle<R>, gateway: G, clock: C) -> Arc<Self> {
        let registry = Arc::new(SurfaceRegistry::default());
        let capabilities = SurfaceCapabilities::detect(|key| std::env::var(key).ok());
        Arc::new_cyclic(|weak| Self {
            downloads: Arc::new(DownloadDispatcher::new(
                registry.clone(),
                gateway.clone(),
                app.clone(),
                clock,
            )),
            windowed: WindowedAdapter::new(app.clone()),
            #[cfg(feature = "embedded-surfaces")]
            embedded: crate::surface::embedded::EmbeddedAdapter::new(app.clone()),
            idle: IdleTimers::default(),
            pushes: PushSequencer::default(),
            capabilities,
            weak: weak.clone(),
            registry,
            gateway,
            app,
        })
    }

    /// What this build and display can do.
    pub fn capabilities(&self) -> SurfaceCapabilities {
        self.capabilities
    }

    /// Every live instance, ordered by id.
    pub fn list(&self) -> Vec<SurfaceRecord> {
        self.registry.snapshot()
    }

    /// Opens a surface or focuses the existing singleton instance.
    ///
    /// An embedded request on a build or display without embedded support is downgraded to
    /// windowed; the returned record carries the target that was actually used.
    pub fn open(
        &self,
        plugin_id: &PluginId,
        surface_id: &str,
        target: MountTarget,
    ) -> Result<SurfaceRecord, SurfaceError> {
        let definition = self.definition(plugin_id, surface_id)?;
        let target = match target {
            MountTarget::Embedded if !self.capabilities.embedded => MountTarget::Windowed,
            MountTarget::Embedded | MountTarget::Windowed => target,
        };
        self.idle.disarm(plugin_id);
        let (record, effects) = match self.registry.open(definition, target) {
            Ok(opened) => opened,
            Err(OpenError::AlreadyOpen(existing)) => {
                self.focus(&existing);
                return Ok(*existing);
            }
        };
        self.execute(record.instance, effects);
        Ok(self.registry.record(record.instance).unwrap_or(record))
    }

    /// Closes one instance; a close that is already in flight is accepted silently.
    pub fn close(&self, instance: SurfaceInstanceId) -> Result<(), SurfaceError> {
        self.command(instance, SurfaceCommand::Close)
    }

    /// Shows or hides an embedded instance; other mounts ignore the request.
    pub fn set_visible(
        &self,
        instance: SurfaceInstanceId,
        visible: bool,
    ) -> Result<(), SurfaceError> {
        match self.state_of(instance)? {
            SurfaceState::Embedded { .. } => {
                self.command(instance, SurfaceCommand::SetVisible(visible))
            }
            SurfaceState::Opening { .. }
            | SurfaceState::Windowed { .. }
            | SurfaceState::Migrating { .. }
            | SurfaceState::Closing { .. }
            | SurfaceState::Failed { .. } => Ok(()),
        }
    }

    /// Moves an embedded instance to the placeholder rectangle; other mounts ignore the request.
    pub fn set_bounds(
        &self,
        instance: SurfaceInstanceId,
        position: LogicalPosition<f64>,
        size: LogicalSize<f64>,
    ) -> Result<(), SurfaceError> {
        let SurfaceState::Embedded { .. } = self.state_of(instance)? else {
            return Ok(());
        };
        let Some(webview) = self.webview_of(instance) else {
            return Ok(());
        };
        webview
            .set_position(position)
            .and_then(|_| webview.set_size(size))
            .map_err(|error| SurfaceError::Internal(format!("failed to move surface: {error}")))
    }

    /// Reloads the current page of an instance, or rebuilds a failed one.
    ///
    /// A failed instance has no webview (its creation is what failed), so a page reload cannot
    /// recover it; the frontend's retry therefore maps onto the registry's `Rebuild` transition,
    /// which re-runs `CreateWebview` and emits `opened` on success.
    pub fn reload(&self, instance: SurfaceInstanceId) -> Result<(), SurfaceError> {
        if let SurfaceState::Failed { .. } = self.state_of(instance)? {
            return self.command(instance, SurfaceCommand::Rebuild);
        }
        let webview = self
            .webview_of(instance)
            .ok_or(SurfaceError::InstanceNotFound(instance))?;
        webview
            .reload()
            .map_err(|error| SurfaceError::Internal(format!("failed to reload surface: {error}")))
    }

    /// Closes every instance of one plugin and cancels its idle timer; used before the plugin
    /// is stopped, disabled, or uninstalled.
    pub fn close_all(&self, plugin_id: &PluginId) {
        self.idle.disarm(plugin_id);
        for record in self.registry.instances_of(plugin_id) {
            if let Err(error) = self.close(record.instance) {
                ora_warn!(message = "failed to close surface of plugin", plugin_id = %plugin_id, instance = record.instance.value(), error = %error);
            }
        }
    }

    /// Closes every instance of every plugin; used when the main window goes away.
    pub fn close_everything(&self) {
        for record in self.registry.snapshot() {
            let _ = self.close(record.instance);
        }
    }

    /// Applies a registry command and executes its effects.
    pub(super) fn command(
        &self,
        instance: SurfaceInstanceId,
        command: SurfaceCommand,
    ) -> Result<(), SurfaceError> {
        let effects = self.registry.command(instance, command)?;
        self.execute(instance, effects);
        Ok(())
    }

    /// Current state of one instance.
    pub(super) fn state_of(
        &self,
        instance: SurfaceInstanceId,
    ) -> Result<SurfaceState, SurfaceError> {
        self.registry
            .record(instance)
            .map(|record| record.state)
            .ok_or(SurfaceError::InstanceNotFound(instance))
    }

    /// Resolves the manifest surface of an installed, enabled plugin.
    fn definition(
        &self,
        plugin_id: &PluginId,
        surface_id: &str,
    ) -> Result<SurfaceDefinition, SurfaceError> {
        let surfaces = self
            .gateway
            .installed_surfaces(plugin_id)
            .ok_or_else(|| SurfaceError::PluginNotFound(plugin_id.clone()))?;
        if !self.gateway.plugin_enabled(plugin_id) {
            return Err(SurfaceError::PluginDisabled(plugin_id.clone()));
        }
        surfaces
            .iter()
            .find(|surface| surface.id.as_str() == surface_id)
            .map(|surface| SurfaceDefinition::from_installed(plugin_id, surface))
            .ok_or_else(|| SurfaceError::SurfaceNotFound {
                plugin_id: plugin_id.clone(),
                surface_id: surface_id.to_owned(),
            })
    }

    /// Brings an existing windowed instance to the front; embedded instances are shown by the
    /// frontend through `surface_set_visible`.
    fn focus(&self, record: &SurfaceRecord) {
        let Some(window) = self.app.get_webview_window(record.label.as_str()) else {
            return;
        };
        if let Err(error) = window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
        {
            ora_warn!(message = "failed to focus surface window", label = %record.label, error = %error);
        }
    }
}

/// Adapter that lets the plugin lifecycle close surfaces before stopping a plugin.
pub struct SurfaceCloserHandle<G: SurfacePluginGateway, R: Runtime, C: DownloadClock>(
    pub Arc<SurfaceService<G, R, C>>,
);

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock + Send + Sync + 'static> SurfaceCloser
    for SurfaceCloserHandle<G, R, C>
{
    fn close_all(&self, plugin_id: &PluginId) -> impl Future<Output = ()> + Send {
        let service = self.0.clone();
        let plugin_id = plugin_id.clone();
        async move { service.close_all(&plugin_id) }
    }
}

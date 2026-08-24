//! Execution of registry effects against real webviews; the only place that mutates Tauri
//! windows on behalf of the surface state machine.

use crate::surface::gateway::SurfacePluginGateway;
use crate::surface::hooks::{SurfaceHooks, SystemBrowserOpener};
use crate::surface::idle::{IDLE_GRACE, IdleOutcome, wait_for_idle};
use crate::surface::service::SurfaceService;
use crate::surface::spec::{AdapterError, Placement, SurfaceAdapter, SurfaceWebviewSpec};
use crate::surface::web_data::{self, HostPlatform};
use crate::surface::{MAIN_WINDOW_LABEL, SURFACE_EVENT};
use ora_logging::{ora_info, ora_warn};
use ora_surface::{
    MountTarget, SurfaceCompletion, SurfaceEffect, SurfaceEvent, SurfaceInstanceId, SurfaceRecord,
};
use std::collections::VecDeque;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview, WindowEvent};

/// Sends one surface event to the main webview; failures are logged because the registry, not
/// the UI, is the source of truth.
pub fn emit_event<R: Runtime>(app: &AppHandle<R>, event: &SurfaceEvent) {
    if let Err(error) = app.emit_to(MAIN_WINDOW_LABEL, SURFACE_EVENT, event) {
        ora_warn!(message = "failed to emit surface event", event = ?event, error = %error);
    }
}

impl<G: SurfacePluginGateway, R: Runtime> SurfaceService<G, R> {
    /// Executes effects in order, feeding completions back into the registry until the queue
    /// drains. No registry lock is held while a webview is touched.
    pub(super) fn execute(&self, instance: SurfaceInstanceId, effects: Vec<SurfaceEffect>) {
        let mut queue: VecDeque<SurfaceEffect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            match effect {
                SurfaceEffect::Emit(event) => emit_event(&self.app, &event),
                SurfaceEffect::CreateWebview { target, operation } => {
                    let outcome = self.create(instance, target).map(|_| target);
                    queue.extend(self.complete(
                        instance,
                        SurfaceCompletion::Opened {
                            operation,
                            outcome: outcome.map_err(|error| error.to_string()),
                        },
                    ));
                }
                SurfaceEffect::DestroyWebview { operation } => {
                    let record = self.registry.record(instance);
                    self.destroy(instance);
                    queue.extend(self.complete(instance, SurfaceCompletion::Closed { operation }));
                    if let Some(record) = record {
                        self.after_closed(record);
                    }
                }
                SurfaceEffect::Reparent { to, operation } => {
                    let outcome = self.reparent(instance, to).map(|_| to);
                    queue.extend(self.complete(
                        instance,
                        SurfaceCompletion::Migrated {
                            operation,
                            outcome: outcome.map_err(|error| error.to_string()),
                        },
                    ));
                }
                SurfaceEffect::SetNativeVisibility(visible) => {
                    if let Some(webview) = self.webview_of(instance) {
                        let result = if visible {
                            webview.show()
                        } else {
                            webview.hide()
                        };
                        if let Err(error) = result {
                            ora_warn!(message = "failed to change surface visibility", instance = instance.value(), error = %error);
                        }
                    }
                }
            }
        }
    }

    /// Applies a completion and logs stale tickets instead of failing.
    fn complete(
        &self,
        instance: SurfaceInstanceId,
        completion: SurfaceCompletion,
    ) -> Vec<SurfaceEffect> {
        self.registry
            .complete(instance, completion)
            .unwrap_or_else(|error| {
                ora_warn!(message = "surface completion ignored", instance = instance.value(), error = %error);
                vec![]
            })
    }

    /// Creates the webview for an `Opening` instance on the requested target.
    fn create(&self, instance: SurfaceInstanceId, target: MountTarget) -> Result<(), String> {
        let record = self
            .registry
            .record(instance)
            .ok_or_else(|| "instance vanished before its webview was created".to_owned())?;
        let plugin_id = &record.definition.plugin_id;
        let plugin_data = self
            .gateway
            .data_directory(plugin_id)
            .map_err(|error| error.to_string())?;
        // Both kinds get one persistent profile per plugin: a webview plugin keeps its login
        // state, and a workbench page keeps its `localStorage` separate from other plugins on
        // platforms where all `ora-plugin://` pages share one origin.
        let web_data = web_data::resolve(plugin_id, &plugin_data, HostPlatform::CURRENT)
            .map_err(|error| format!("failed to prepare the web profile: {error}"))?;
        let spec = SurfaceWebviewSpec::new(&record, web_data)
            .map_err(|error| format!("failed to build the surface URL: {error}"))?;
        let hooks = SurfaceHooks::new(
            spec.label.clone(),
            spec.navigation.clone(),
            self.downloads.clone(),
            Arc::new(SystemBrowserOpener),
        );
        let webview = match target {
            MountTarget::Windowed => self.windowed.create(&spec, hooks, Placement::Windowed),
            MountTarget::Embedded => self.create_embedded(&spec, hooks),
        }
        .map_err(|error| error.to_string())?;
        if target == MountTarget::Windowed {
            self.watch_window(&webview, instance);
        }
        ora_info!(message = "surface webview created", label = %spec.label, target = ?target);
        Ok(())
    }

    #[cfg(feature = "embedded-surfaces")]
    fn create_embedded(
        &self,
        spec: &SurfaceWebviewSpec,
        hooks: SurfaceHooks<
            crate::surface::downloads::DownloadDispatcher<G, R>,
            SystemBrowserOpener,
        >,
    ) -> Result<Webview<R>, AdapterError> {
        self.embedded.create(spec, hooks, Placement::parked())
    }

    /// Without the feature no embedded instance can exist; `open` already downgraded the target,
    /// so reaching this means a logic error that surfaces as a failed instance, not a panic.
    #[cfg(not(feature = "embedded-surfaces"))]
    fn create_embedded(
        &self,
        _spec: &SurfaceWebviewSpec,
        _hooks: SurfaceHooks<
            crate::surface::downloads::DownloadDispatcher<G, R>,
            SystemBrowserOpener,
        >,
    ) -> Result<Webview<R>, AdapterError> {
        Err(AdapterError::EmbeddedUnsupported)
    }

    /// Turns the user closing a surface window into a registry `Close` command.
    fn watch_window(&self, webview: &Webview<R>, instance: SurfaceInstanceId) {
        let service = self.weak.clone();
        webview.window().on_window_event(move |event| {
            if let WindowEvent::CloseRequested { .. } = event
                && let Some(service) = service.upgrade()
            {
                // The close may already be in flight (idempotent) or the instance may be gone
                // (unknown); neither is an error from the window's point of view.
                let _ = service.close(instance);
            }
        });
    }

    /// Destroys whatever native object currently hosts the instance.
    fn destroy(&self, instance: SurfaceInstanceId) {
        let Some(record) = self.registry.record(instance) else {
            return;
        };
        let label = record.label.as_str();
        let result = if let Some(window) = self.app.get_webview_window(label) {
            // `destroy` skips the close-requested round trip, which would otherwise re-enter
            // the registry with a second `Close`.
            window.destroy()
        } else if let Some(webview) = self.find_webview(label) {
            webview.close()
        } else {
            Ok(())
        };
        if let Err(error) = result {
            ora_warn!(message = "failed to destroy surface webview", label, error = %error);
        }
        #[cfg(feature = "embedded-surfaces")]
        self.destroy_popout_window(label);
    }

    /// Notifies the plugin and arms the idle timer once a plugin has no instance left.
    fn after_closed(&self, record: SurfaceRecord) {
        let plugin_id = record.definition.plugin_id.clone();
        if !self.registry.instances_of(&plugin_id).is_empty() {
            return;
        }
        let token = self.idle.arm(&plugin_id);
        let gateway = self.gateway.clone();
        let registry = self.registry.clone();
        tauri::async_runtime::spawn(async move {
            if wait_for_idle(token, IDLE_GRACE).await == IdleOutcome::Cancelled {
                return;
            }
            // The instance count is re-checked because an open that raced the timer expiry
            // may not have disarmed it yet.
            if !registry.instances_of(&plugin_id).is_empty() {
                return;
            }
            match gateway.stop_if_idle(&plugin_id).await {
                Ok(()) => {
                    ora_info!(message = "idle plugin process stopped", plugin_id = %plugin_id)
                }
                Err(error) => {
                    ora_warn!(message = "failed to stop idle plugin process", plugin_id = %plugin_id, error = %error)
                }
            }
        });
    }

    /// Returns the webview hosting an instance, whichever mount it currently has.
    pub(super) fn webview_of(&self, instance: SurfaceInstanceId) -> Option<Webview<R>> {
        let record = self.registry.record(instance)?;
        self.find_webview(record.label.as_str())
    }

    /// Looks a webview up by label. `Manager::get_webview` (which also sees child webviews) is
    /// unstable API, so the stable build only knows webview windows.
    pub(super) fn find_webview(&self, label: &str) -> Option<Webview<R>> {
        #[cfg(feature = "embedded-surfaces")]
        {
            self.app.get_webview(label)
        }
        #[cfg(not(feature = "embedded-surfaces"))]
        {
            self.app
                .get_webview_window(label)
                .map(|window| window.as_ref().clone())
        }
    }
}

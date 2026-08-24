//! Moving an instance between mounts: `Webview::reparent` with the `embedded-surfaces` feature,
//! destroy-and-rebuild without it.

use crate::surface::error::SurfaceError;
use crate::surface::gateway::SurfacePluginGateway;
use crate::surface::service::SurfaceService;
use ora_surface::{MountTarget, SurfaceInstanceId};
use tauri::Runtime;

impl<G: SurfacePluginGateway, R: Runtime> SurfaceService<G, R> {
    /// Moves an embedded instance into its own window.
    ///
    /// Without child-webview support the registry never holds an embedded instance, so the
    /// fallback closes the instance and reopens the same definition windowed; the frontend sees
    /// `closed` followed by `opened` with a new instance id instead of `migrated`.
    pub fn popout(&self, instance: SurfaceInstanceId) -> Result<(), SurfaceError> {
        #[cfg(feature = "embedded-surfaces")]
        {
            self.command(instance, ora_surface::SurfaceCommand::Popout)
        }
        #[cfg(not(feature = "embedded-surfaces"))]
        {
            let record = self
                .registry
                .record(instance)
                .ok_or(SurfaceError::InstanceNotFound(instance))?;
            self.close(instance)?;
            let plugin_id = record.definition.plugin_id.clone();
            self.open(&plugin_id, MountTarget::Windowed).map(|_| ())
        }
    }

    /// Moves a windowed instance back into the main window; requires child-webview support.
    pub fn dock(&self, instance: SurfaceInstanceId) -> Result<(), SurfaceError> {
        #[cfg(feature = "embedded-surfaces")]
        {
            self.command(instance, ora_surface::SurfaceCommand::Dock)
        }
        #[cfg(not(feature = "embedded-surfaces"))]
        {
            let _ = instance;
            Err(SurfaceError::Unsupported("surface_dock"))
        }
    }

    /// Reparents the instance's webview to the target mount.
    #[cfg(feature = "embedded-surfaces")]
    pub(super) fn reparent(
        &self,
        instance: SurfaceInstanceId,
        to: MountTarget,
    ) -> Result<(), String> {
        use crate::surface::MAIN_WINDOW_LABEL;
        use tauri::{LogicalPosition, LogicalSize, Manager, WindowEvent};

        let record = self
            .registry
            .record(instance)
            .ok_or_else(|| "instance vanished before migration".to_owned())?;
        let webview = self
            .find_webview(record.label.as_str())
            .ok_or_else(|| "surface webview is missing".to_owned())?;
        match to {
            MountTarget::Windowed => {
                // A plain (webview-less) window hosts the child; its label derives from the
                // surface label so `destroy_popout_window` can find it again.
                let window = tauri::window::WindowBuilder::new(
                    &self.app,
                    popout_window_label(record.label.as_str()),
                )
                .title(&record.definition.title)
                .inner_size(1100.0, 760.0)
                .min_inner_size(720.0, 520.0)
                .center()
                .build()
                .map_err(|error| error.to_string())?;
                webview
                    .reparent(&window)
                    .map_err(|error| error.to_string())?;
                let size = window.inner_size().map_err(|error| error.to_string())?;
                let scale = window.scale_factor().unwrap_or(1.0);
                webview
                    .set_position(LogicalPosition::new(0.0, 0.0))
                    .and_then(|_| webview.set_size(size.to_logical::<f64>(scale)))
                    .map_err(|error| error.to_string())?;
                let resized = webview.clone();
                let service = self.weak.clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::Resized(size) => {
                        let _ = resized.set_size(*size);
                    }
                    WindowEvent::CloseRequested { .. } => {
                        if let Some(service) = service.upgrade() {
                            let _ = service.close(instance);
                        }
                    }
                    _ => {}
                });
                Ok(())
            }
            MountTarget::Embedded => {
                let main = self
                    .app
                    .get_window(MAIN_WINDOW_LABEL)
                    .ok_or_else(|| "main window is unavailable".to_owned())?;
                webview.reparent(&main).map_err(|error| error.to_string())?;
                // Park until the frontend sends the placeholder bounds.
                let _ = webview.set_size(LogicalSize::new(1.0, 1.0));
                self.destroy_popout_window(record.label.as_str());
                Ok(())
            }
        }
    }

    /// Migration is impossible without the feature; the registry reports `MigrateFailed`.
    #[cfg(not(feature = "embedded-surfaces"))]
    pub(super) fn reparent(
        &self,
        _instance: SurfaceInstanceId,
        _to: MountTarget,
    ) -> Result<(), String> {
        Err("embedded surfaces are not compiled into this build".to_owned())
    }

    /// Destroys the plain window created by a popout, if one exists.
    #[cfg(feature = "embedded-surfaces")]
    pub(super) fn destroy_popout_window(&self, label: &str) {
        use tauri::Manager;
        if let Some(window) = self.app.get_window(&popout_window_label(label))
            && let Err(error) = window.destroy()
        {
            ora_logging::ora_warn!(message = "failed to destroy popout window", label, error = %error);
        }
    }
}

/// Label of the plain window hosting a popped-out child webview.
#[cfg(feature = "embedded-surfaces")]
fn popout_window_label(label: &str) -> String {
    format!("{label}/window")
}

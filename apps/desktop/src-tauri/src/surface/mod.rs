//! Desktop host for plugin UI surfaces: the only module that touches Tauri webview APIs on
//! behalf of `ora-surface`. See `README.md` in this directory.

mod capabilities;
pub mod commands;
pub mod download_actions;
mod downloads;
mod effects;
#[cfg(feature = "embedded-surfaces")]
mod embedded;
mod error;
mod gateway;
mod hooks;
mod idle;
mod migrate;
mod service;
mod spec;
#[cfg(test)]
mod tests;
mod web_data;
mod windowed;
mod workbench_assets;
pub mod workbench_bridge;

pub use service::SurfaceService;
pub use workbench_assets::register_protocol as register_workbench_protocol;

use ora_backend::PluginGateway;
use ora_logging::ora_info;
use service::SurfaceCloserHandle;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WindowEvent};

/// Label of the application webview that owns the frontend and receives surface events.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// Event channel carrying `ora_surface::SurfaceEvent` payloads to the frontend.
pub const SURFACE_EVENT: &str = "surface://event";

/// The production service: backend gateway plus the Wry runtime.
pub type DesktopSurfaceService = SurfaceService<Arc<PluginGateway>, tauri::Wry>;

/// Connects the service to the process lifecycle and to the main window.
///
/// Registering the closer makes disable/uninstall close surfaces before the plugin process
/// stops; the main window destroy hook closes every surface so no orphan window outlives the
/// frontend that controls it; the download action host lets automatic download dispositions run
/// their host action (skill import) without a frontend round trip.
pub fn install(
    app: &AppHandle,
    service: &Arc<DesktopSurfaceService>,
    backend: &ora_backend::Backend,
) {
    service
        .gateway
        .set_surface_closer(SurfaceCloserHandle(service.clone()));
    service.install_download_action_host(Arc::new(backend.clone()));
    if let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let service = Arc::downgrade(service);
        main.on_window_event(move |event| {
            if let WindowEvent::Destroyed = event
                && let Some(service) = service.upgrade()
            {
                service.close_everything();
            }
        });
    }
    ora_info!(
        message = "surface service installed",
        embedded = service.capabilities().embedded
    );
}

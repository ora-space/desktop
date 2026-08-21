//! Desktop host for plugin UI surfaces: the only module that touches Tauri webview APIs on
//! behalf of `ora-surface`. See `README.md` in this directory.

pub mod bridge;
mod capabilities;
pub mod commands;
mod downloads;
mod effects;
#[cfg(feature = "embedded-surfaces")]
mod embedded;
mod error;
mod gateway;
mod hooks;
mod idle;
mod migrate;
mod panel_assets;
#[cfg(test)]
mod panel_tests;
mod plugin_link;
mod push;
mod service;
mod spec;
#[cfg(test)]
mod tests;
mod web_data;
mod windowed;

pub use panel_assets::register_protocol as register_panel_protocol;
pub use service::SurfaceService;

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
/// frontend that controls it; the push router forwards plugin notifications to panels for as
/// long as the service lives.
pub fn install(app: &AppHandle, service: &Arc<DesktopSurfaceService>) {
    service
        .gateway
        .set_surface_closer(SurfaceCloserHandle(service.clone()));
    let router = service.clone();
    tauri::async_runtime::spawn(async move { router.route_pushes().await });
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

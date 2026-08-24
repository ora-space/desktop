//! Embedded adapter: child webviews of the main window (`Window::add_child`).
//!
//! Only compiled with the `embedded-surfaces` feature, which enables Tauri's `unstable` API.
//! Bounds arrive from the frontend in CSS pixels; Tauri logical units are CSS pixels already,
//! so they are passed through as `LogicalPosition`/`LogicalSize` and the device scale factor
//! the frontend reports is informational only.

use crate::surface::MAIN_WINDOW_LABEL;
use crate::surface::hooks::{DownloadSink, PopupOpener, SurfaceHooks, apply_spec};
use crate::surface::spec::{AdapterError, Placement, SurfaceAdapter, SurfaceWebviewSpec};
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, Webview};

/// Creates child webviews inside the main window.
pub struct EmbeddedAdapter<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> EmbeddedAdapter<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> SurfaceAdapter<R> for EmbeddedAdapter<R> {
    fn create<D: DownloadSink<R>, P: PopupOpener>(
        &self,
        spec: &SurfaceWebviewSpec,
        hooks: SurfaceHooks<D, P>,
        placement: Placement,
    ) -> Result<Webview<R>, AdapterError> {
        let (position, size) = match placement {
            Placement::Embedded { position, size } => (position, size),
            // A windowed placement carries no rectangle; park until `surface_set_bounds`.
            Placement::Windowed => match Placement::parked() {
                Placement::Embedded { position, size } => (position, size),
                Placement::Windowed => (LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0)),
            },
        };
        let main_window = self
            .app
            .get_window(MAIN_WINDOW_LABEL)
            .ok_or(AdapterError::MainWindowMissing)?;
        let builder = WebviewBuilder::new(spec.label.as_str(), spec.webview_url());
        let builder = apply_spec(hooks.attach(builder), spec);
        main_window
            .add_child(builder, position, size)
            .map_err(AdapterError::Create)
    }
}

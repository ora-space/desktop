//! Windowed adapter: one `WebviewWindow` per surface instance (stable Tauri API).

use crate::surface::hooks::{DownloadSink, PopupOpener, SurfaceHooks, apply_spec};
use crate::surface::spec::{AdapterError, Placement, SurfaceAdapter, SurfaceWebviewSpec};
use tauri::{AppHandle, Runtime, Webview, WebviewWindowBuilder};

/// Default window size; matches the former marketplace window so existing users see no change.
const INNER_SIZE: (f64, f64) = (1100.0, 760.0);
const MIN_INNER_SIZE: (f64, f64) = (720.0, 520.0);

/// Creates surface windows through `WebviewWindowBuilder`.
pub struct WindowedAdapter<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> WindowedAdapter<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> SurfaceAdapter<R> for WindowedAdapter<R> {
    fn create<D: DownloadSink<R>, P: PopupOpener>(
        &self,
        spec: &SurfaceWebviewSpec,
        hooks: SurfaceHooks<D, P>,
        placement: Placement,
    ) -> Result<Webview<R>, AdapterError> {
        // The placement rectangle only matters for embedded surfaces; a windowed surface always
        // opens centered so it is discoverable even on a secondary monitor layout.
        match placement {
            Placement::Windowed => {}
            #[cfg(feature = "embedded-surfaces")]
            Placement::Embedded { .. } => {}
        }
        let builder = WebviewWindowBuilder::new(&self.app, spec.label.as_str(), spec.webview_url())
            .title(&spec.title)
            .inner_size(INNER_SIZE.0, INNER_SIZE.1)
            .min_inner_size(MIN_INNER_SIZE.0, MIN_INNER_SIZE.1)
            .center();
        let builder = apply_spec(hooks.attach(builder), spec);
        let window = builder.build().map_err(AdapterError::Create)?;
        Ok(window.as_ref().clone())
    }
}

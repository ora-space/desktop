//! Immutable build parameters of one surface webview, derived from the registry record.

use crate::surface::web_data::ResolvedWebData;
use ora_surface::{NavigationPolicy, SurfaceRecord, SurfaceSource, WebviewLabel};
#[cfg(feature = "embedded-surfaces")]
use tauri::{LogicalPosition, LogicalSize};
use tauri::{Runtime, Url, Webview};
use thiserror::Error;

/// Everything an adapter needs to build the webview; no Tauri handles, so it is testable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceWebviewSpec {
    pub label: WebviewLabel,
    pub url: Url,
    pub navigation: NavigationPolicy,
    pub web_data: ResolvedWebData,
    /// Window title for windowed surfaces; embedded surfaces have no title bar.
    pub title: String,
}

impl SurfaceWebviewSpec {
    /// Projects a registry record plus the resolved web data into build parameters.
    pub fn new(record: &SurfaceRecord, web_data: ResolvedWebData) -> Self {
        let SurfaceSource::RemoteSite(site) = &record.definition.source;
        Self {
            label: record.label.clone(),
            url: site.entry_url.clone(),
            navigation: site.navigation.clone(),
            web_data,
            title: record.definition.title.clone(),
        }
    }
}

/// Where the webview is mounted when created.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// Its own native window, centered with the default size.
    Windowed,
    /// A child of the main window at the given logical rectangle.
    #[cfg(feature = "embedded-surfaces")]
    Embedded {
        position: LogicalPosition<f64>,
        size: LogicalSize<f64>,
    },
}

#[cfg(feature = "embedded-surfaces")]
impl Placement {
    /// A 1x1 child at the origin, used until the frontend reports the real placeholder bounds.
    pub fn parked() -> Self {
        Self::Embedded {
            position: LogicalPosition::new(0.0, 0.0),
            size: LogicalSize::new(1.0, 1.0),
        }
    }
}

/// Why the platform refused to create or place a webview.
#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("failed to create the surface webview: {0}")]
    Create(#[source] tauri::Error),
    #[cfg(feature = "embedded-surfaces")]
    #[error("the main window is unavailable")]
    MainWindowMissing,
    #[cfg(not(feature = "embedded-surfaces"))]
    #[error("embedded surfaces are not compiled into this build")]
    EmbeddedUnsupported,
}

/// Creates webviews for one mount target.
///
/// Implementations apply the same navigation/download hooks and web data settings; only the
/// builder path (window vs child webview) differs. `create` is synchronous because Tauri's
/// builders block on the event loop themselves.
pub trait SurfaceAdapter<R: Runtime> {
    /// Builds the webview described by `spec` and returns its handle.
    fn create<D: crate::surface::hooks::DownloadSink<R>>(
        &self,
        spec: &SurfaceWebviewSpec,
        hooks: crate::surface::hooks::SurfaceHooks<D>,
        placement: Placement,
    ) -> Result<Webview<R>, AdapterError>;
}

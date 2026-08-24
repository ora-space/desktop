//! Immutable build parameters of one surface webview, derived from the registry record.

use crate::surface::web_data::ResolvedWebData;
use ora_surface::{
    AssetUrlForm, NavigationPolicy, SurfaceRecord, SurfaceSource, WebviewLabel, asset_base,
    entry_url,
};

/// Injected into every workbench webview; defines `window.ora`.
pub const WORKBENCH_API_SCRIPT: &str = include_str!("workbench_api.js");
#[cfg(feature = "embedded-surfaces")]
use tauri::{LogicalPosition, LogicalSize};
use tauri::{Runtime, Url, Webview, WebviewUrl};
use thiserror::Error;
use url::ParseError;

/// Everything an adapter needs to build the webview; no Tauri handles, so it is testable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceWebviewSpec {
    pub label: WebviewLabel,
    pub url: Url,
    pub navigation: NavigationPolicy,
    pub web_data: ResolvedWebData,
    /// Script run before the page; workbench pages receive the bridge API, remote sites nothing.
    pub initialization_script: Option<&'static str>,
    /// Window title for windowed surfaces; embedded surfaces have no title bar.
    pub title: String,
}

impl SurfaceWebviewSpec {
    /// Projects a registry record plus the resolved web data into build parameters.
    ///
    /// The source decides everything but the web data: a remote site starts at its start URL
    /// inside its origin allow list; a workbench page starts at its host-served entry document,
    /// may only navigate below its own asset base, and gets the bridge API injected. Building the
    /// asset URLs is fallible only in theory (instances are integers); the error still surfaces as
    /// a failed instance instead of a panic.
    pub fn new(record: &SurfaceRecord, web_data: ResolvedWebData) -> Result<Self, ParseError> {
        let (url, navigation, initialization_script) = match &record.definition.source {
            SurfaceSource::RemoteSite(site) => {
                (site.start_url.clone(), site.navigation.clone(), None)
            }
            SurfaceSource::Workbench(workbench) => {
                let instance = record.instance;
                (
                    entry_url(AssetUrlForm::CURRENT, instance, workbench)?,
                    NavigationPolicy::workbench_assets(asset_base(
                        AssetUrlForm::CURRENT,
                        instance,
                    )?),
                    Some(WORKBENCH_API_SCRIPT),
                )
            }
        };
        Ok(Self {
            label: record.label.clone(),
            url,
            navigation,
            web_data,
            initialization_script,
            title: record.definition.title.clone(),
        })
    }

    /// The builder URL: Tauri distinguishes `http(s)` from custom schemes at the type level.
    pub fn webview_url(&self) -> WebviewUrl {
        match self.url.scheme() {
            "http" | "https" => WebviewUrl::External(self.url.clone()),
            _ => WebviewUrl::CustomProtocol(self.url.clone()),
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
    fn create<D: crate::surface::hooks::DownloadSink<R>, P: crate::surface::hooks::PopupOpener>(
        &self,
        spec: &SurfaceWebviewSpec,
        hooks: crate::surface::hooks::SurfaceHooks<D, P>,
        placement: Placement,
    ) -> Result<Webview<R>, AdapterError>;
}

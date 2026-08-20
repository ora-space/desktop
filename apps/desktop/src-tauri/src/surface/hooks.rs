//! Navigation, new-window, and download hook factories shared by both adapters.
//!
//! `WebviewBuilder` and `WebviewWindowBuilder` expose identical hook methods without a common
//! trait, so `SurfaceBuilder` is the minimal local glue that lets one `SurfaceHooks::attach`
//! serve both paths and guarantees the two mount targets enforce the same policy.

use crate::surface::web_data::ResolvedWebData;
use ora_logging::ora_info;
use ora_surface::{NavigationPolicy, WebviewLabel};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::webview::{DownloadEvent, NewWindowFeatures, NewWindowResponse};
use tauri::{Manager, Runtime, Url, Webview, WebviewWindowBuilder};

/// Receives download events from a surface webview and decides whether they proceed.
///
/// Implementations resolve the label through the registry (never trusting it as authority),
/// reserve destinations, and dispatch completions; the hook only forwards.
pub trait DownloadSink<R: Runtime>: Send + Sync + 'static {
    /// Handles one download event for the webview `label`; `page_url` is the page that
    /// triggered it when the runtime can report one.
    fn handle(&self, label: &WebviewLabel, page_url: Option<Url>, event: DownloadEvent<'_>)
    -> bool;
}

/// The builder methods both Tauri builders share, abstracted for static dispatch.
pub trait SurfaceBuilder<R: Runtime>: Sized {
    fn on_navigation<F: Fn(&Url) -> bool + Send + 'static>(self, handler: F) -> Self;
    fn on_new_window<F: Fn(Url, NewWindowFeatures) -> NewWindowResponse<R> + Send + 'static>(
        self,
        handler: F,
    ) -> Self;
    fn on_download<F: Fn(Webview<R>, DownloadEvent<'_>) -> bool + Send + Sync + 'static>(
        self,
        handler: F,
    ) -> Self;
    fn data_directory(self, directory: PathBuf) -> Self;
    fn incognito(self, incognito: bool) -> Self;
    fn data_store_identifier(self, identifier: [u8; 16]) -> Self;
}

/// `WebviewBuilder` is only public with Tauri's `unstable` feature.
#[cfg(feature = "embedded-surfaces")]
impl<R: Runtime> SurfaceBuilder<R> for tauri::webview::WebviewBuilder<R> {
    fn on_navigation<F: Fn(&Url) -> bool + Send + 'static>(self, handler: F) -> Self {
        tauri::webview::WebviewBuilder::on_navigation(self, handler)
    }

    fn on_new_window<F: Fn(Url, NewWindowFeatures) -> NewWindowResponse<R> + Send + 'static>(
        self,
        handler: F,
    ) -> Self {
        tauri::webview::WebviewBuilder::on_new_window(self, handler)
    }

    fn on_download<F: Fn(Webview<R>, DownloadEvent<'_>) -> bool + Send + Sync + 'static>(
        self,
        handler: F,
    ) -> Self {
        tauri::webview::WebviewBuilder::on_download(self, handler)
    }

    fn data_directory(self, directory: PathBuf) -> Self {
        tauri::webview::WebviewBuilder::data_directory(self, directory)
    }

    fn incognito(self, incognito: bool) -> Self {
        tauri::webview::WebviewBuilder::incognito(self, incognito)
    }

    fn data_store_identifier(self, identifier: [u8; 16]) -> Self {
        tauri::webview::WebviewBuilder::data_store_identifier(self, identifier)
    }
}

impl<R: Runtime, M: Manager<R>> SurfaceBuilder<R> for WebviewWindowBuilder<'_, R, M> {
    fn on_navigation<F: Fn(&Url) -> bool + Send + 'static>(self, handler: F) -> Self {
        WebviewWindowBuilder::on_navigation(self, handler)
    }

    fn on_new_window<F: Fn(Url, NewWindowFeatures) -> NewWindowResponse<R> + Send + 'static>(
        self,
        handler: F,
    ) -> Self {
        WebviewWindowBuilder::on_new_window(self, handler)
    }

    fn on_download<F: Fn(Webview<R>, DownloadEvent<'_>) -> bool + Send + Sync + 'static>(
        self,
        handler: F,
    ) -> Self {
        WebviewWindowBuilder::on_download(self, handler)
    }

    fn data_directory(self, directory: PathBuf) -> Self {
        WebviewWindowBuilder::data_directory(self, directory)
    }

    fn incognito(self, incognito: bool) -> Self {
        WebviewWindowBuilder::incognito(self, incognito)
    }

    fn data_store_identifier(self, identifier: [u8; 16]) -> Self {
        WebviewWindowBuilder::data_store_identifier(self, identifier)
    }
}

/// The policy closures attached to one surface webview.
pub struct SurfaceHooks<D> {
    label: WebviewLabel,
    navigation: NavigationPolicy,
    downloads: Arc<D>,
}

impl<D> SurfaceHooks<D> {
    /// Bundles the label, navigation policy, and download sink of one instance.
    pub fn new(label: WebviewLabel, navigation: NavigationPolicy, downloads: Arc<D>) -> Self {
        Self {
            label,
            navigation,
            downloads,
        }
    }

    /// Installs the hooks on a builder of either kind. Popups inside the allow list open with
    /// the default system behaviour, exactly like the former marketplace window.
    pub fn attach<R: Runtime, B: SurfaceBuilder<R>>(self, builder: B) -> B
    where
        D: DownloadSink<R>,
    {
        let navigation = self.navigation.clone();
        let popups = self.navigation;
        let downloads = self.downloads;
        let label = self.label;
        let navigation_label = label.clone();
        let popup_label = label.clone();
        builder
            .on_navigation(move |url| {
                let allowed = navigation.allows(url);
                // Denials are the only navigation outcome worth recording: a remote site that
                // silently stops working is otherwise impossible to diagnose from the logs.
                if !allowed {
                    ora_info!(message = "surface navigation denied", label = %navigation_label, url = %url);
                }
                allowed
            })
            .on_new_window(move |url, _features| {
                if popups.allows(&url) {
                    NewWindowResponse::Allow
                } else {
                    ora_info!(message = "surface popup denied", label = %popup_label, url = %url);
                    NewWindowResponse::Deny
                }
            })
            .on_download(move |webview, event| {
                // The page URL is informational (it lands in `ui/downloadCompleted`), so a
                // runtime that cannot report it degrades to `None` instead of failing.
                downloads.handle(&label, webview.url().ok(), event)
            })
    }
}

/// Applies the resolved web data mechanism to a builder of either kind.
pub fn apply_web_data<R: Runtime, B: SurfaceBuilder<R>>(
    builder: B,
    web_data: &ResolvedWebData,
) -> B {
    match web_data {
        ResolvedWebData::Directory(directory) => builder.data_directory(directory.clone()),
        ResolvedWebData::StoreIdentifier(identifier) => builder.data_store_identifier(*identifier),
        ResolvedWebData::Incognito => builder.incognito(true),
        ResolvedWebData::SharedDefault => builder,
    }
}

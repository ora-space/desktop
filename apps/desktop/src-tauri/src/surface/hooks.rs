//! Navigation, new-window, and download hook factories shared by both adapters.
//!
//! `WebviewBuilder` and `WebviewWindowBuilder` expose identical hook methods without a common
//! trait, so `SurfaceBuilder` is the minimal local glue that lets one `SurfaceHooks::attach`
//! serve both paths and guarantees the two mount targets enforce the same policy.

use crate::open_external::open_external_url_blocking;
use crate::surface::spec::SurfaceWebviewSpec;
use crate::surface::web_data::ResolvedWebData;
use ora_logging::{ora_info, ora_warn};
use ora_surface::{NavigationPolicy, WebviewLabel};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::webview::{DownloadEvent, NewWindowFeatures, NewWindowResponse};
use tauri::{Manager, Runtime, Url, Webview, WebviewWindowBuilder};

/// Opens a popup URL outside every Ora webview.
///
/// A popup that passes the allow list is still handed to the system browser rather than given a
/// webview of its own: Tauri's default `Allow` creates a window with no navigation, download, or
/// registry hooks, so the page could redirect anywhere afterwards and outlive the surface that
/// spawned it. Implementations only receive URLs the surface policy already accepted.
pub trait PopupOpener: Send + Sync + 'static {
    /// Opens `url` in the host's default browser; failures are reported, never retried.
    fn open(&self, url: &Url) -> Result<(), PopupOpenError>;
}

/// Reports that the host browser could not be launched for a popup URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PopupOpenError(String);

/// The production opener: the same OS launch path as the `open_external_url` command.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemBrowserOpener;

impl PopupOpener for SystemBrowserOpener {
    fn open(&self, url: &Url) -> Result<(), PopupOpenError> {
        open_external_url_blocking(url.as_str()).map_err(|error| PopupOpenError(error.to_string()))
    }
}

/// Decides one `window.open` request from a surface page.
///
/// The answer is always `Deny` for the webview: an allowed URL leaves the Ora trust boundary via
/// the system browser, everything else is dropped and logged. A workbench page never needs a
/// second window: its only allowed URLs are its own assets, and opening them outside the page
/// would show a page without its bridge.
pub fn handle_popup<R: Runtime, P: PopupOpener>(
    policy: &NavigationPolicy,
    opener: &P,
    label: &WebviewLabel,
    url: &Url,
) -> NewWindowResponse<R> {
    let allowed = match policy {
        NavigationPolicy::RemoteSite { .. } => policy.allows(url),
        NavigationPolicy::WorkbenchAssets { .. } => false,
    };
    if allowed {
        if let Err(error) = opener.open(url) {
            ora_warn!(message = "surface popup could not open in the browser", label = %label, url = %url, error = %error);
        }
    } else {
        ora_info!(message = "surface popup denied", label = %label, url = %url);
    }
    NewWindowResponse::Deny
}

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
    fn data_store_identifier(self, identifier: [u8; 16]) -> Self;
    fn initialization_script(self, script: &str) -> Self;
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

    fn data_store_identifier(self, identifier: [u8; 16]) -> Self {
        tauri::webview::WebviewBuilder::data_store_identifier(self, identifier)
    }

    fn initialization_script(self, script: &str) -> Self {
        tauri::webview::WebviewBuilder::initialization_script(self, script)
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

    fn data_store_identifier(self, identifier: [u8; 16]) -> Self {
        WebviewWindowBuilder::data_store_identifier(self, identifier)
    }

    fn initialization_script(self, script: &str) -> Self {
        WebviewWindowBuilder::initialization_script(self, script)
    }
}

/// The policy closures attached to one surface webview.
pub struct SurfaceHooks<D, P> {
    label: WebviewLabel,
    navigation: NavigationPolicy,
    downloads: Arc<D>,
    popups: Arc<P>,
}

impl<D, P> SurfaceHooks<D, P> {
    /// Bundles the label, navigation policy, download sink, and popup opener of one instance.
    pub fn new(
        label: WebviewLabel,
        navigation: NavigationPolicy,
        downloads: Arc<D>,
        popups: Arc<P>,
    ) -> Self {
        Self {
            label,
            navigation,
            downloads,
            popups,
        }
    }

    /// Installs the hooks on a builder of either kind. Popups inside the allow list are handed
    /// to the system browser; no popup ever becomes an Ora-hosted webview (see `handle_popup`).
    pub fn attach<R: Runtime, B: SurfaceBuilder<R>>(self, builder: B) -> B
    where
        D: DownloadSink<R>,
        P: PopupOpener,
    {
        let navigation = self.navigation.clone();
        let popups = self.navigation;
        let opener = self.popups;
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
                handle_popup(&popups, opener.as_ref(), &popup_label, &url)
            })
            .on_download(move |webview, event| {
                // The page URL decides the download disposition; a runtime that cannot report
                // it degrades to `None`, and the sink refuses such downloads.
                downloads.handle(&label, webview.url().ok(), event)
            })
    }
}

/// Applies the spec's web data mechanism and optional injected script to a builder of either
/// kind, so both mount targets configure the page identically.
pub fn apply_spec<R: Runtime, B: SurfaceBuilder<R>>(builder: B, spec: &SurfaceWebviewSpec) -> B {
    let builder = match &spec.web_data {
        ResolvedWebData::Directory(directory) => builder.data_directory(directory.clone()),
        ResolvedWebData::StoreIdentifier(identifier) => builder.data_store_identifier(*identifier),
        ResolvedWebData::SharedDefault => builder,
    };
    match spec.initialization_script {
        Some(script) => builder.initialization_script(script),
        None => builder,
    }
}

#[cfg(test)]
mod tests {
    use super::{NewWindowResponse, PopupOpenError, PopupOpener, handle_popup};
    use ora_plugin_manifest::Origin;
    use ora_surface::{NavigationPolicy, SurfaceInstanceId, WebviewLabel};
    use pretty_assertions::assert_eq;
    use std::sync::Mutex;
    use tauri::Url;
    use tauri::test::MockRuntime;

    /// Records every URL handed to the browser instead of launching one.
    #[derive(Default)]
    struct RecordingOpener(Mutex<Vec<String>>);

    impl PopupOpener for RecordingOpener {
        fn open(&self, url: &Url) -> Result<(), PopupOpenError> {
            self.0.lock().expect("opener lock").push(url.to_string());
            Ok(())
        }
    }

    /// Builds a remote-site policy that trusts exactly `https://site.example`.
    fn policy() -> NavigationPolicy {
        NavigationPolicy::remote_site(vec![
            Origin::parse("https://site.example").expect("valid origin"),
        ])
    }

    fn label() -> WebviewLabel {
        WebviewLabel::remote(SurfaceInstanceId::new(1))
    }

    /// Verifies popups never become Ora webviews: an allowed URL goes to the browser and is
    /// denied, a disallowed URL is neither opened nor allowed.
    #[test]
    fn popups_are_denied_and_only_allowed_urls_reach_the_browser() {
        let opener = RecordingOpener::default();
        let allowed = Url::parse("https://site.example/skills/1").expect("url");
        let denied = Url::parse("https://evil.example/").expect("url");

        let responses = [&allowed, &denied]
            .into_iter()
            .map(|url| {
                matches!(
                    handle_popup::<MockRuntime, _>(&policy(), &opener, &label(), url),
                    NewWindowResponse::Deny
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            (responses, opener.0.into_inner().expect("opener lock")),
            (
                vec![true, true],
                vec!["https://site.example/skills/1".to_owned()],
            ),
        );
    }

    /// Verifies a workbench page never opens a popup, even for its own allowed asset URLs.
    #[test]
    fn workbench_pages_never_open_popups() {
        let base = Url::parse("ora-plugin://localhost/7/").expect("url");
        let policy = NavigationPolicy::workbench_assets(base.clone());
        let opener = RecordingOpener::default();

        let denied = matches!(
            handle_popup::<MockRuntime, _>(&policy, &opener, &label(), &base),
            NewWindowResponse::Deny
        );

        assert_eq!(
            (denied, opener.0.into_inner().expect("opener lock")),
            (true, Vec::new())
        );
    }
}

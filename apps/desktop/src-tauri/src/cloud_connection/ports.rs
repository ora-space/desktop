//! The side effects the Cloud connection needs besides HTTP and the secure store, with the
//! production implementations the host composes.

use std::future::Future;

use ora_contracts::CloudConnection;
use tauri::{AppHandle, Emitter, Manager};

use super::event::CHANGED_EVENT;

/// Opens the system browser. Blocking: callers run it off the async workers.
pub(crate) trait BrowserOpener: Send + Sync + 'static {
    /// Opens `url` in the default browser, or explains why that failed.
    fn open(&self, url: &str) -> Result<(), String>;
}

/// Persists the chosen server origin (not secret) next to the other preferences.
pub(crate) trait ServerSetting: Send + Sync + 'static {
    /// The saved origin, if any.
    fn load(&self) -> impl Future<Output = Result<Option<String>, String>> + Send;
    /// Saves `origin`, or forgets the saved one when `None`.
    fn store(&self, origin: Option<String>) -> impl Future<Output = Result<(), String>> + Send;
}

/// Learns about connection changes on behalf of the main webview.
pub(crate) trait ConnectionObserver: Send + Sync + 'static {
    /// Called with the new connection after every change.
    fn changed(&self, connection: &CloudConnection);
    /// Called once a sign-in completed, when the person is still looking at the browser.
    fn signed_in(&self);
}

/// The default browser through the host's existing http(s)-only opener.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemBrowser;

impl BrowserOpener for SystemBrowser {
    /// Reuses the opener the main webview's external links go through.
    fn open(&self, url: &str) -> Result<(), String> {
        crate::open_external::open_external_url_blocking(url).map_err(|error| error.to_string())
    }
}

impl ServerSetting for ora_backend::Settings {
    /// Reads the preference through the backend's settings owner.
    async fn load(&self) -> Result<Option<String>, String> {
        self.cloud_server().await.map_err(|error| error.to_string())
    }

    /// Writes or clears the preference through the backend's settings owner.
    async fn store(&self, origin: Option<String>) -> Result<(), String> {
        match origin {
            Some(origin) => self.set_cloud_server(origin).await,
            None => self.clear_cloud_server().await,
        }
        .map_err(|error| error.to_string())
    }
}

/// Pushes every change to the main webview and brings it forward after a sign-in.
#[derive(Clone)]
pub(crate) struct WebviewObserver {
    app: AppHandle,
}

impl WebviewObserver {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ConnectionObserver for WebviewObserver {
    /// Plugin webviews never receive this event: it is addressed to the main webview only.
    fn changed(&self, connection: &CloudConnection) {
        let _ = self.app.emit_to("main", CHANGED_EVENT, connection);
    }

    /// The browser had focus for the approval; returning focus saves a window switch.
    fn signed_in(&self) {
        if let Some(window) = self.app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

//! Desktop's connection to one self-hosted Cloud server (v2-server).
//!
//! Signing in follows RFC 8252: the host opens the system browser on the server's approval page,
//! receives the redirect on a loopback listener, and exchanges the one-time code with its PKCE
//! verifier for a desktop token. The token stays in this module and the operating system's secure
//! store; the main webview only learns the server, the account name and the state, through the
//! commands in `commands::cloud_connection` and the change event. Local work never depends on any
//! of this.

mod api;
#[path = "../bindings/cloud_connection_event.rs"]
mod event;
mod loopback;
mod pkce;
mod ports;
mod secret;
mod secure_store;
mod server;
mod service;
#[cfg(test)]
mod tests;

use api::HttpCloudApi;
use ports::{SystemBrowser, WebviewObserver};
use secure_store::KeyringStore;
pub(crate) use service::{CloudConnectionService, Ports, Timing};

/// The production composition: OS keyring, HTTP, default browser, backend settings, main webview.
pub(crate) type DesktopCloudConnection = CloudConnectionService<
    KeyringStore,
    HttpCloudApi,
    SystemBrowser,
    ora_backend::Settings,
    WebviewObserver,
>;

/// Starts the connection for the running application; restoring happens in the background.
pub(crate) fn start(
    app: tauri::AppHandle,
    settings: ora_backend::Settings,
) -> DesktopCloudConnection {
    CloudConnectionService::start(
        Ports {
            store: KeyringStore,
            api: HttpCloudApi,
            browser: SystemBrowser,
            setting: settings,
            observer: WebviewObserver::new(app),
        },
        Timing::default(),
        tauri::async_runtime::handle().inner().clone(),
    )
}

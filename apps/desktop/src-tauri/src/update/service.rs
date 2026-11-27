//! Wires the Desktop update engine to the Tauri updater and registers the checks that drive it.

use super::artifact_store::UpdateArtifactStore;
use super::engine::{OfferedPackage, UpdateEngine, UpdateHost};
use super::job::UpdateJob;
use super::platform::{InstallSupport, install_support};
use super::verifier::UpdateVerifier;
use super::{DesktopUpdateStatus, UpdateError};
use ora_backend::Settings;
use ora_logging::ora_error;
use ora_scheduler::{CronHandle, DelayHandle, Scheduler};
use ora_utils::http::{HostProxy, HostProxyError, is_proxy_auth_failure};
use semver::Version;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

const UPDATE_EVENT: &str = "desktop-update-status-changed";
const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(60);

/// Selects whether the Desktop composition root should register release update work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopUpdateMode {
    /// Register the delayed and recurring release update checks.
    Enabled,
    /// Keep commands available but do not perform network work in development builds.
    Disabled,
}

/// Drives update checks, downloads, recovery, and installation for the Desktop application.
#[derive(Clone)]
pub struct UpdateService {
    inner: Arc<UpdateServiceInner>,
}

struct UpdateServiceInner {
    engine: UpdateEngine<TauriUpdateHost, UpdateVerifier>,
    _scheduler: Scheduler,
    _initial_check: Mutex<Option<DelayHandle>>,
    _cron: Mutex<Option<CronHandle>>,
}

impl UpdateService {
    /// Creates the service, opens the recoverable artifact store, and registers update checks.
    pub fn start(
        app: AppHandle,
        settings: Settings,
        home_directory: &Path,
        timezone: chrono_tz::Tz,
        mode: DesktopUpdateMode,
    ) -> Result<Self, UpdateError> {
        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0));
        let artifacts = UpdateArtifactStore::open(home_directory, &current)?;
        let verifier = UpdateVerifier::from_app(&app)?;
        let scheduler = Scheduler::new(timezone);
        let service = Self {
            inner: Arc::new(UpdateServiceInner {
                engine: UpdateEngine::new(TauriUpdateHost { app, settings }, artifacts, verifier),
                _scheduler: scheduler.clone(),
                _initial_check: Mutex::new(None),
                _cron: Mutex::new(None),
            }),
        };

        if matches!(mode, DesktopUpdateMode::Enabled) {
            let delayed_service = service.clone();
            let initial = scheduler
                .schedule_after(INITIAL_CHECK_DELAY, async move {
                    delayed_service.check_and_download().await;
                })
                .map_err(UpdateError::Scheduler)?;
            let cron = scheduler
                .schedule_cron(UpdateJob::new(service.clone()))
                .map_err(UpdateError::Scheduler)?;
            *service
                .inner
                ._initial_check
                .lock()
                .expect("update initial handle mutex is not poisoned") = Some(initial);
            *service
                .inner
                ._cron
                .lock()
                .expect("update cron handle mutex is not poisoned") = Some(cron);
        }
        Ok(service)
    }

    /// Returns the latest status snapshot for a command or a freshly mounted frontend.
    pub fn status(&self) -> DesktopUpdateStatus {
        self.inner.engine.status()
    }

    /// Checks for a release and downloads it, keeping a still-verified ready package on failure.
    pub async fn check_and_download(&self) {
        self.inner.engine.check_and_download().await;
    }

    /// Installs the confirmed release, then restarts where the platform installer does not.
    pub async fn install(&self, version: &str) -> Result<(), UpdateError> {
        self.inner.engine.install(version).await?;
        // Windows hands control to the NSIS updater, which terminates this process itself; Linux
        // replaces the AppImage in place and has to be restarted here.
        #[cfg(target_os = "windows")]
        {
            Ok(())
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.engine.host().app.restart()
        }
    }
}

/// The Tauri updater plugin, the packaged version, and the webview event bridge.
struct TauriUpdateHost {
    app: AppHandle,
    settings: Settings,
}

impl UpdateHost for TauriUpdateHost {
    type Release = RoutedUpdate;

    fn running_version(&self) -> Version {
        self.app.package_info().version.clone()
    }

    fn install_support(&self) -> InstallSupport {
        install_support()
    }

    async fn check(&self) -> Result<Option<RoutedUpdate>, UpdateError> {
        let policy = self
            .settings
            .network_proxy_policy()
            .await
            .map_err(|error| UpdateError::ProxySettings(error.to_string()))?;
        let config = updater_config(&self.app)?;
        for url in &config.endpoints {
            policy.route(url)?;
        }
        let route = policy.clone();
        let updater = self
            .app
            .updater_builder()
            .no_proxy()
            .configure_client(move |builder| {
                crate::network_proxy::configure_client(builder, &route)
            })
            .build()
            .map_err(UpdateError::Updater)?;
        updater
            .check()
            .await
            .map_err(network_update_error)
            .map(|update| update.map(|update| RoutedUpdate { update, policy }))
    }

    fn publish(&self, status: DesktopUpdateStatus) {
        if let Err(error) = self.app.emit(UPDATE_EVENT, status) {
            ora_error!(message = "failed to publish Desktop update status", error = %error);
        }
    }
}

/// Gives the settings probe the updater's configured manifest endpoint.
pub(crate) fn probe_url(app: &AppHandle) -> Option<String> {
    updater_config(app)
        .ok()?
        .endpoints
        .first()
        .map(ToString::to_string)
}

/// Reads the release transport configuration from its Tauri source of truth.
fn updater_config(app: &AppHandle) -> Result<tauri_plugin_updater::Config, UpdateError> {
    let config = app
        .config()
        .plugins
        .0
        .get("updater")
        .ok_or(UpdateError::Updater(
            tauri_plugin_updater::Error::EmptyEndpoints,
        ))?;
    serde_json::from_value(config.clone()).map_err(|error| UpdateError::Updater(error.into()))
}

/// Keeps the manifest's routing snapshot with the package that it offered.
#[derive(Clone)]
struct RoutedUpdate {
    update: Update,
    policy: HostProxy,
}

impl OfferedPackage for RoutedUpdate {
    fn version(&self) -> &str {
        &self.update.version
    }

    fn target(&self) -> &str {
        &self.update.target
    }

    fn download_url(&self) -> &Url {
        &self.update.download_url
    }

    fn signature(&self) -> &str {
        &self.update.signature
    }

    fn raw_json(&self) -> &Value {
        &self.update.raw_json
    }

    async fn download(
        &self,
        on_chunk: impl FnMut(usize, Option<u64>) + Send,
    ) -> Result<Vec<u8>, UpdateError> {
        self.policy.route(&self.update.download_url)?;
        self.update
            .download(on_chunk, || {})
            .await
            .map_err(network_update_error)
    }

    fn install(&self, bytes: Vec<u8>) -> Result<(), UpdateError> {
        self.update.install(bytes).map_err(UpdateError::Updater)
    }
}

/// Reports proxy authentication failures without using a credential-bearing error URL.
fn network_update_error(error: tauri_plugin_updater::Error) -> UpdateError {
    if is_proxy_auth_failure(&error)
        || matches!(&error, tauri_plugin_updater::Error::Network(message)
        if message.starts_with("Download request failed with status: 407"))
    {
        return UpdateError::Proxy(HostProxyError::Authentication);
    }
    UpdateError::Updater(error)
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// Mirrors the updater's ring initialization before building a test transport.
    fn client_for(policy: &HostProxy) -> updater_http::Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::network_proxy::configure_client(updater_http::Client::builder(), policy)
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }

    /// Captures a request at the configured proxy and returns a controlled HTTP response.
    async fn proxy_exchange(listener: TcpListener, response: String) -> String {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(&mut socket);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).await.unwrap() == 0 || header == "\r\n" {
                break;
            }
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        request
    }

    /// The manifest hop reaches the proxy, then HTTPS to loopback starts TLS directly.
    #[tokio::test]
    async fn updater_client_reroutes_redirect_to_loopback() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let policy = HostProxy::manual(
            format!("http://{}", proxy.local_addr().unwrap())
                .parse()
                .unwrap(),
        );
        let target = direct.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: https://{target}/package\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let client = client_for(&policy);
        let accept_direct = async {
            let (mut socket, _) = direct.accept().await.unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), 0x16, "TLS handshake");
        };
        let (request, _, client_result) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                proxy_exchange(proxy, response),
                accept_direct,
                client
                    .get("http://manifest.example.test/update.json")
                    .send()
            )
        })
        .await
        .unwrap();
        assert!(request.starts_with("GET http://manifest.example.test/update.json "));
        assert!(
            client_result.is_err(),
            "fixture closes the socket during TLS"
        );
    }

    /// The same updater transport reports an actual CONNECT 407 as a proxy auth failure.
    #[tokio::test]
    async fn updater_client_classifies_connect_407() {
        for version in ["HTTP/1.0", "HTTP/1.1"] {
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let policy = HostProxy::manual(
                format!("http://{}", proxy.local_addr().unwrap())
                    .parse()
                    .unwrap(),
            );
            let response = format!(
                "{version} 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=ora\r\nContent-Length: 0\r\n\r\n"
            );
            let client = client_for(&policy);
            let (request, result) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(
                    proxy_exchange(proxy, response),
                    client
                        .get("https://manifest.example.test/update.json")
                        .send()
                )
            })
            .await
            .unwrap();
            assert!(request.starts_with("CONNECT manifest.example.test:443 "));
            assert!(matches!(
                network_update_error(result.unwrap_err().into()),
                UpdateError::Proxy(HostProxyError::Authentication)
            ));
        }
        assert!(matches!(
            network_update_error(tauri_plugin_updater::Error::Network(
                "Download request failed with status: 407 Proxy Authentication Required".to_owned()
            )),
            UpdateError::Proxy(HostProxyError::Authentication)
        ));
    }
}

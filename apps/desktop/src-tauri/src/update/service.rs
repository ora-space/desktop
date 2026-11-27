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
        let config = self
            .app
            .config()
            .plugins
            .0
            .get("updater")
            .ok_or(UpdateError::Updater(
                tauri_plugin_updater::Error::EmptyEndpoints,
            ))?;
        let config: tauri_plugin_updater::Config = serde_json::from_value(config.clone())
            .map_err(|error| UpdateError::Updater(error.into()))?;
        for url in &config.endpoints {
            policy.route(url)?;
        }
        let route = policy.clone();
        let updater = self
            .app
            .updater_builder()
            .no_proxy()
            .configure_client(move |builder| configure_proxy_client(builder, &route))
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

/// Applies the same policy to Tauri's manifest and package clients and to each redirect.
fn configure_proxy_client(
    builder: updater_http::ClientBuilder,
    policy: &HostProxy,
) -> updater_http::ClientBuilder {
    use updater_http::{Proxy, redirect::Policy};
    let proxy = policy.clone();
    let redirect = policy.clone();
    let limit = Policy::limited(10);
    builder
        .no_proxy()
        .proxy(Proxy::custom(move |target| {
            proxy.route(target).ok()?.endpoint
        }))
        .redirect(Policy::custom(move |attempt| {
            if attempt.url().scheme() != "https"
                || !attempt.url().username().is_empty()
                || attempt.url().password().is_some()
            {
                return attempt.error("unsafe update redirect");
            }
            if let Err(error) = redirect.route(attempt.url()) {
                return attempt.error(error);
            }
            limit.redirect(attempt)
        }))
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

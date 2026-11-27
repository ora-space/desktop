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
    type Release = Update;

    fn running_version(&self) -> Version {
        self.app.package_info().version.clone()
    }

    fn install_support(&self) -> InstallSupport {
        install_support()
    }

    async fn check(&self) -> Result<Option<Update>, UpdateError> {
        let mut updater_builder = self.app.updater_builder();
        if let Some(settings) = self
            .settings
            .network_proxy_settings()
            .await
            .map_err(|error| UpdateError::ProxySettings(error.to_string()))?
        {
            updater_builder = updater_builder.proxy(proxy_url(&settings)?);
        }
        let updater = updater_builder.build().map_err(UpdateError::Updater)?;
        updater.check().await.map_err(UpdateError::Updater)
    }

    fn publish(&self, status: DesktopUpdateStatus) {
        if let Err(error) = self.app.emit(UPDATE_EVENT, status) {
            ora_error!(message = "failed to publish Desktop update status", error = %error);
        }
    }
}

impl OfferedPackage for Update {
    fn version(&self) -> &str {
        &self.version
    }

    fn target(&self) -> &str {
        &self.target
    }

    fn download_url(&self) -> &Url {
        &self.download_url
    }

    fn signature(&self) -> &str {
        &self.signature
    }

    fn raw_json(&self) -> &Value {
        &self.raw_json
    }

    async fn download(
        &self,
        on_chunk: impl FnMut(usize, Option<u64>) + Send,
    ) -> Result<Vec<u8>, UpdateError> {
        Update::download(self, on_chunk, || {})
            .await
            .map_err(UpdateError::Updater)
    }

    fn install(&self, bytes: Vec<u8>) -> Result<(), UpdateError> {
        Update::install(self, bytes).map_err(UpdateError::Updater)
    }
}

/// Converts persisted host proxy settings into the URL accepted by Tauri updater.
pub(super) fn proxy_url(
    settings: &ora_application::NetworkProxySettings,
) -> Result<Url, UpdateError> {
    let mut url = Url::parse("http://proxy.invalid").map_err(UpdateError::Proxy)?;
    url.set_host(Some(&settings.host))
        .map_err(|_| UpdateError::ProxyCredentials)?;
    url.set_port(Some(settings.port))
        .map_err(|_| UpdateError::ProxyCredentials)?;
    if let Some(username) = &settings.username {
        url.set_username(username)
            .map_err(|_| UpdateError::ProxyCredentials)?;
    }
    if let Some(password) = &settings.password {
        url.set_password(Some(password))
            .map_err(|_| UpdateError::ProxyCredentials)?;
    }
    Ok(url)
}

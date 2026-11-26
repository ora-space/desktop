//! Owns the Desktop updater state machine and the scheduler registrations that drive it.

use super::artifact_store::{ArtifactDescriptor, UpdateArtifactStore};
use super::job::UpdateJob;
use super::platform::{InstallSupport, install_support};
use super::state::{OfferedRelease, RuntimeUpdateState};
use super::verifier::UpdateVerifier;
use super::{DesktopUpdateStatus, UpdateError};
use ora_backend::Settings;
use ora_logging::{ora_error, ora_info, ora_warn};
use ora_scheduler::{CronHandle, DelayHandle, Scheduler};
use ora_utils::error::error_chain;
use ora_utils::http::{HostProxy, HostProxyError, is_proxy_auth_failure};
use semver::Version;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex as AsyncMutex;

const UPDATE_EVENT: &str = "desktop-update-status-changed";
const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(60);
/// Progress is emitted per megabyte: a per-chunk event would flood the webview bridge without
/// moving a progress bar any further than this does.
const PROGRESS_EVENT_INTERVAL: u64 = 1024 * 1024;

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
    app: AppHandle,
    settings: Settings,
    artifacts: UpdateArtifactStore,
    verifier: UpdateVerifier,
    state: Mutex<RuntimeUpdateState<Update>>,
    operation: AsyncMutex<()>,
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
                app,
                settings,
                artifacts,
                verifier,
                state: Mutex::new(RuntimeUpdateState::Current),
                operation: AsyncMutex::new(()),
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
        self.inner
            .state
            .lock()
            .expect("update state mutex is not poisoned")
            .status()
    }

    /// Checks for a release and recovers matching verified bytes before spending another download.
    ///
    /// A package already installable in this process survives a failed replacement check. A package
    /// from an earlier process is advertised only after a fresh manifest supplies its installer
    /// handle and signature again.
    pub async fn check_and_download(&self) {
        let _operation = self.inner.operation.lock().await;
        let retained = self
            .inner
            .state
            .lock()
            .expect("update state mutex is not poisoned")
            .ready();
        if retained.is_none() {
            self.set_state(RuntimeUpdateState::Checking);
        }
        if let Err(error) = self.check_and_download_inner().await {
            ora_warn!(
                message = "Desktop update check failed",
                error = %error_chain(&error),
            );
            let retained_is_valid = match retained.as_ref() {
                Some(ready) => self
                    .inner
                    .artifacts
                    .read_verified(&ready.artifact, &ready.descriptor, &self.inner.verifier)
                    .await
                    .is_ok(),
                None => false,
            };
            self.set_state(RuntimeUpdateState::after_failed_check(
                retained,
                retained_is_valid,
                &error,
            ));
        }
    }

    /// Installs the confirmed release after re-reading its record and signature from disk.
    pub async fn install(&self, version: &str) -> Result<(), UpdateError> {
        let _operation = self.inner.operation.lock().await;
        let ready = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("update state mutex is not poisoned");
            let ready = state.begin_install(version)?;
            let status = state.status();
            drop(state);
            self.emit_status(status);
            ready
        };

        let bytes = match self
            .inner
            .artifacts
            .read_verified(&ready.artifact, &ready.descriptor, &self.inner.verifier)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) => {
                self.set_state(RuntimeUpdateState::Failed {
                    message: error_chain(&error),
                });
                return Err(error);
            }
        };
        if let Err(error) = ready.installer.install(bytes) {
            self.set_state(RuntimeUpdateState::Ready(ready));
            return Err(UpdateError::Updater(error));
        }
        self.set_state(RuntimeUpdateState::Current);
        self.inner.artifacts.clear();
        // Windows hands control to the NSIS updater, which terminates this process itself; Linux
        // replaces the AppImage in place and has to be restarted here.
        #[cfg(target_os = "windows")]
        {
            Ok(())
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.app.restart()
        }
    }

    /// Reconciles the current manifest with an existing artifact or a newly verified download.
    async fn check_and_download_inner(&self) -> Result<(), UpdateError> {
        let policy = self
            .inner
            .settings
            .network_proxy_policy()
            .await
            .map_err(|error| UpdateError::ProxySettings(error.to_string()))?;
        let config =
            self.inner
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
            .inner
            .app
            .updater_builder()
            .no_proxy()
            .configure_client(move |builder| configure_proxy_client(builder, &route))
            .build()
            .map_err(UpdateError::Updater)?;
        let Some(update) = updater.check().await.map_err(network_update_error)? else {
            self.inner.artifacts.clear();
            self.set_state(RuntimeUpdateState::Current);
            return Ok(());
        };

        // Built from the manifest of this very check, so a status never shows another release's notes.
        let offered = OfferedRelease::from_manifest(
            &update.raw_json,
            &self.inner.app.package_info().version,
            update.version.clone(),
        );
        if let InstallSupport::Manual(reason) = install_support() {
            self.inner.artifacts.clear();
            ora_info!(
                message = "Desktop update requires a manual installation",
                version = %update.version,
            );
            self.set_state(offered.manual(reason));
            return Ok(());
        }

        let descriptor = ArtifactDescriptor::new(
            update.version.clone(),
            update.target.clone(),
            update.download_url.clone(),
            update.signature.clone(),
            &self.inner.verifier,
        )?;
        if let Some(artifact) = self
            .inner
            .artifacts
            .find_verified(&descriptor, &self.inner.verifier)
            .await?
        {
            let version = descriptor.identity.release_version.clone();
            self.set_state(offered.ready(update, descriptor, artifact));
            ora_info!(message = "Desktop update recovered from cache", version = %version);
            return Ok(());
        }

        self.set_state(RuntimeUpdateState::Downloading {
            version: update.version.clone(),
            downloaded: 0,
            total: None,
        });
        policy.route(&update.download_url)?;
        let bytes = self.download(&update).await?;
        let artifact = self
            .inner
            .artifacts
            .commit(&descriptor, &bytes, &self.inner.verifier)
            .await?;
        let version = descriptor.identity.release_version.clone();
        self.set_state(offered.ready(update, descriptor, artifact));
        ora_info!(message = "Desktop update downloaded", version = %version);
        Ok(())
    }

    /// Downloads the signed package while publishing throttled progress to the webview.
    async fn download(&self, update: &Update) -> Result<Vec<u8>, UpdateError> {
        let service = self.clone();
        let version = update.version.clone();
        let mut downloaded = 0u64;
        let mut published = 0u64;
        update
            .download(
                move |chunk_length, content_length| {
                    downloaded += chunk_length as u64;
                    let complete = content_length == Some(downloaded);
                    if !complete && downloaded - published < PROGRESS_EVENT_INTERVAL {
                        return;
                    }
                    published = downloaded;
                    service.set_state(RuntimeUpdateState::Downloading {
                        version: version.clone(),
                        downloaded,
                        total: content_length,
                    });
                },
                || {},
            )
            .await
            .map_err(network_update_error)
    }

    /// Replaces the complete runtime state and publishes its derived webview status.
    fn set_state(&self, state: RuntimeUpdateState<Update>) {
        let status = state.status();
        *self
            .inner
            .state
            .lock()
            .expect("update state mutex is not poisoned") = state;
        self.emit_status(status);
    }

    /// Publishes a status snapshot without making event delivery mandatory for state progress.
    fn emit_status(&self, status: DesktopUpdateStatus) {
        if let Err(error) = self.inner.app.emit(UPDATE_EVENT, status) {
            ora_error!(message = "failed to publish Desktop update status", error = %error);
        }
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

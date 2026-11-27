//! Runs update checks, downloads, recovery, and installation against an abstract updater host.
//!
//! The Tauri updater, the running package's version, and the webview event bridge sit behind
//! [`UpdateHost`], so the wiring between one check's manifest, the running version, and the state
//! a failure leaves behind can be tested without an application handle.

use super::artifact_store::{ArtifactDescriptor, ArtifactVerifier, UpdateArtifactStore};
use super::platform::InstallSupport;
use super::state::{OfferedRelease, RuntimeUpdateState};
use super::{DesktopUpdateStatus, UpdateError};
use ora_logging::{ora_info, ora_warn};
use ora_utils::error::error_chain;
use semver::Version;
use serde_json::Value;
use std::future::Future;
use std::sync::Mutex;
use tokio::sync::Mutex as AsyncMutex;
use url::Url;

/// Progress is emitted per megabyte: a per-chunk event would flood the webview bridge without
/// moving a progress bar any further than this does.
const PROGRESS_EVENT_INTERVAL: u64 = 1024 * 1024;

/// The updater, platform, and webview services one update operation needs from its host.
pub(super) trait UpdateHost: Send + Sync {
    /// The release a check found, which can also download and install its own package.
    type Release: OfferedPackage;

    /// The version of the running package; notes at or below it are not offered again.
    fn running_version(&self) -> Version;

    /// Whether this installation may replace itself with a downloaded package.
    fn install_support(&self) -> InstallSupport;

    /// Fetches the manifest once; `None` means no newer release is offered.
    fn check(&self) -> impl Future<Output = Result<Option<Self::Release>, UpdateError>> + Send;

    /// Delivers a status snapshot to the webview; delivery is best effort.
    fn publish(&self, status: DesktopUpdateStatus);
}

/// One release from a manifest, with its signed package.
pub(super) trait OfferedPackage: Clone + Send + Sync {
    /// The release version the manifest offers.
    fn version(&self) -> &str;

    /// The platform target the manifest entry was selected for.
    fn target(&self) -> &str;

    /// Where the package is downloaded from.
    fn download_url(&self) -> &Url;

    /// The manifest's signature for the package.
    fn signature(&self) -> &str;

    /// The whole manifest of the check that produced this release.
    fn raw_json(&self) -> &Value;

    /// Downloads the package, reporting each chunk's length and the total when known.
    fn download(
        &self,
        on_chunk: impl FnMut(usize, Option<u64>) + Send,
    ) -> impl Future<Output = Result<Vec<u8>, UpdateError>> + Send;

    /// Hands verified bytes to the platform installer.
    fn install(&self, bytes: Vec<u8>) -> Result<(), UpdateError>;
}

/// Owns the update state machine and serializes the operations that change it.
pub(super) struct UpdateEngine<H: UpdateHost, V> {
    host: H,
    artifacts: UpdateArtifactStore,
    verifier: V,
    state: Mutex<RuntimeUpdateState<H::Release>>,
    operation: AsyncMutex<()>,
}

impl<H: UpdateHost, V: ArtifactVerifier + Send + Sync> UpdateEngine<H, V> {
    /// Starts from a current state; a verified package from an earlier process is offered again
    /// only after a fresh check supplies its manifest.
    pub(super) fn new(host: H, artifacts: UpdateArtifactStore, verifier: V) -> Self {
        Self {
            host,
            artifacts,
            verifier,
            state: Mutex::new(RuntimeUpdateState::Current),
            operation: AsyncMutex::new(()),
        }
    }

    /// Returns the host, for platform work that follows an operation such as a restart.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code))]
    pub(super) fn host(&self) -> &H {
        &self.host
    }

    /// Returns the latest status snapshot for a command or a freshly mounted frontend.
    pub(super) fn status(&self) -> DesktopUpdateStatus {
        self.state
            .lock()
            .expect("update state mutex is not poisoned")
            .status()
    }

    /// Checks for a release and recovers matching verified bytes before spending another download.
    ///
    /// A package already installable in this process survives a failed replacement check while its
    /// bytes still verify. A package from an earlier process is advertised only after a fresh
    /// manifest supplies its installer handle and signature again.
    pub(super) async fn check_and_download(&self) {
        let _operation = self.operation.lock().await;
        let retained = self
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
            let verified = match retained {
                Some(ready) => self
                    .artifacts
                    .read_verified(&ready.artifact, &ready.descriptor, &self.verifier)
                    .await
                    .is_ok()
                    .then_some(ready),
                None => None,
            };
            self.set_state(RuntimeUpdateState::after_failed_check(verified, &error));
        }
    }

    /// Installs the confirmed release after re-reading its record and signature from disk.
    ///
    /// Returns once the platform installer has accepted the package; restarting is the host's job.
    pub(super) async fn install(&self, version: &str) -> Result<(), UpdateError> {
        let _operation = self.operation.lock().await;
        let ready = {
            let mut state = self
                .state
                .lock()
                .expect("update state mutex is not poisoned");
            let ready = state.begin_install(version)?;
            let status = state.status();
            drop(state);
            self.host.publish(status);
            ready
        };

        let bytes = match self
            .artifacts
            .read_verified(&ready.artifact, &ready.descriptor, &self.verifier)
            .await
        {
            Ok(bytes) => bytes,
            Err(error) => {
                self.set_state(RuntimeUpdateState::after_failed_install(
                    ready.descriptor.identity.release_version.clone(),
                    &error,
                ));
                return Err(error);
            }
        };
        if let Err(error) = ready.installer.install(bytes) {
            self.set_state(RuntimeUpdateState::Ready(ready));
            return Err(error);
        }
        self.set_state(RuntimeUpdateState::Current);
        self.artifacts.clear();
        Ok(())
    }

    /// Reconciles the current manifest with an existing artifact or a newly verified download.
    async fn check_and_download_inner(&self) -> Result<(), UpdateError> {
        let Some(update) = self.host.check().await? else {
            self.artifacts.clear();
            self.set_state(RuntimeUpdateState::Current);
            return Ok(());
        };

        // Built from the manifest of this very check, so a status never shows another release's notes.
        let offered = OfferedRelease::from_manifest(
            update.raw_json(),
            &self.host.running_version(),
            update.version().to_owned(),
        );
        if let InstallSupport::Manual(reason) = self.host.install_support() {
            self.artifacts.clear();
            ora_info!(
                message = "Desktop update requires a manual installation",
                version = %update.version(),
            );
            self.set_state(offered.manual(reason));
            return Ok(());
        }

        let descriptor = ArtifactDescriptor::new(
            update.version().to_owned(),
            update.target().to_owned(),
            update.download_url().clone(),
            update.signature().to_owned(),
            &self.verifier,
        )?;
        if let Some(artifact) = self
            .artifacts
            .find_verified(&descriptor, &self.verifier)
            .await?
        {
            let version = descriptor.identity.release_version.clone();
            self.set_state(offered.ready(update, descriptor, artifact));
            ora_info!(message = "Desktop update recovered from cache", version = %version);
            return Ok(());
        }

        self.set_state(RuntimeUpdateState::Downloading {
            version: update.version().to_owned(),
            downloaded: 0,
            total: None,
        });
        let bytes = self.download(&update).await?;
        let artifact = self
            .artifacts
            .commit(&descriptor, &bytes, &self.verifier)
            .await?;
        let version = descriptor.identity.release_version.clone();
        self.set_state(offered.ready(update, descriptor, artifact));
        ora_info!(message = "Desktop update downloaded", version = %version);
        Ok(())
    }

    /// Downloads the signed package while publishing throttled progress to the webview.
    async fn download(&self, update: &H::Release) -> Result<Vec<u8>, UpdateError> {
        let version = update.version().to_owned();
        let mut downloaded = 0u64;
        let mut published = 0u64;
        update
            .download(move |chunk_length, content_length| {
                downloaded += chunk_length as u64;
                let complete = content_length == Some(downloaded);
                if !complete && downloaded - published < PROGRESS_EVENT_INTERVAL {
                    return;
                }
                published = downloaded;
                self.set_state(RuntimeUpdateState::Downloading {
                    version: version.clone(),
                    downloaded,
                    total: content_length,
                });
            })
            .await
    }

    /// Replaces the complete runtime state and publishes its derived webview status.
    fn set_state(&self, state: RuntimeUpdateState<H::Release>) {
        let status = state.status();
        *self
            .state
            .lock()
            .expect("update state mutex is not poisoned") = state;
        self.host.publish(status);
    }
}

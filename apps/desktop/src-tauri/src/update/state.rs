//! Runtime state machine for one Desktop update operation.

use super::release_notes::visible_notes;
use super::{DesktopUpdateStatus, FailedOperation, ManualUpdateReason, ReleaseNote, UpdateError};
use crate::update::artifact_store::{ArtifactDescriptor, StoredArtifact};
use ora_utils::error::error_chain;
use semver::Version;
use serde_json::Value;

/// Binds an installer handle to the exact verified artifact it is allowed to install.
#[derive(Clone)]
pub(super) struct ReadyUpdate<I> {
    pub(super) installer: I,
    pub(super) descriptor: ArtifactDescriptor,
    pub(super) artifact: StoredArtifact,
    pub(super) notes: Vec<ReleaseNote>,
}

/// One check's offer, which keeps a release and the notes of its own manifest together.
pub(super) struct OfferedRelease {
    version: String,
    notes: Vec<ReleaseNote>,
}

impl OfferedRelease {
    /// Reads the notes between the running version and the one a manifest offers.
    pub(super) fn from_manifest(manifest: &Value, running: &Version, version: String) -> Self {
        let notes = visible_notes(manifest, running, &version);
        Self { version, notes }
    }

    /// Offers a release that this installation cannot install itself.
    pub(super) fn manual<I>(self, reason: ManualUpdateReason) -> RuntimeUpdateState<I> {
        RuntimeUpdateState::ManualUpdate {
            version: self.version,
            reason,
            notes: self.notes,
        }
    }

    /// Offers a release whose package has been verified.
    pub(super) fn ready<I>(
        self,
        installer: I,
        descriptor: ArtifactDescriptor,
        artifact: StoredArtifact,
    ) -> RuntimeUpdateState<I> {
        RuntimeUpdateState::Ready(ReadyUpdate {
            installer,
            descriptor,
            artifact,
            notes: self.notes,
        })
    }
}

/// Keeps the webview status and installable data in one state so they cannot diverge.
pub(super) enum RuntimeUpdateState<I> {
    Current,
    Checking,
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    Ready(ReadyUpdate<I>),
    ManualUpdate {
        version: String,
        reason: ManualUpdateReason,
        notes: Vec<ReleaseNote>,
    },
    Installing(ReadyUpdate<I>),
    Failed {
        operation: FailedOperation,
        message: String,
    },
}

impl<I: Clone> RuntimeUpdateState<I> {
    /// Projects the internal state into the stable wire contract consumed by the webview.
    pub(super) fn status(&self) -> DesktopUpdateStatus {
        match self {
            Self::Current => DesktopUpdateStatus::Current,
            Self::Checking => DesktopUpdateStatus::Checking,
            Self::Downloading {
                version,
                downloaded,
                total,
            } => DesktopUpdateStatus::Downloading {
                version: version.clone(),
                downloaded: *downloaded,
                total: *total,
            },
            Self::Ready(ready) => DesktopUpdateStatus::Ready {
                version: ready.descriptor.identity.release_version.clone(),
                notes: ready.notes.clone(),
            },
            Self::ManualUpdate {
                version,
                reason,
                notes,
            } => DesktopUpdateStatus::ManualUpdate {
                version: version.clone(),
                reason: *reason,
                notes: notes.clone(),
            },
            Self::Installing(ready) => DesktopUpdateStatus::Installing {
                version: ready.descriptor.identity.release_version.clone(),
            },
            Self::Failed { operation, message } => DesktopUpdateStatus::Failed {
                operation: operation.clone(),
                message: message.clone(),
            },
        }
    }

    /// Returns an installable update that a failed replacement check must preserve.
    pub(super) fn ready(&self) -> Option<ReadyUpdate<I>> {
        match self {
            Self::Ready(ready) => Some(ready.clone()),
            Self::Current
            | Self::Checking
            | Self::Downloading { .. }
            | Self::ManualUpdate { .. }
            | Self::Installing(_)
            | Self::Failed { .. } => None,
        }
    }

    /// Decides what a failed check leaves behind: an update whose package still verifies keeps its
    /// notes, anything else reports the failure.
    ///
    /// The caller passes only a retained update it has re-verified, so a discarded package cannot be
    /// offered again by mistake.
    pub(super) fn after_failed_check(
        verified: Option<ReadyUpdate<I>>,
        error: &UpdateError,
    ) -> Self {
        match verified {
            Some(ready) => Self::Ready(ready),
            None => Self::Failed {
                operation: FailedOperation::Check,
                message: error_chain(error),
            },
        }
    }

    /// Reports that a confirmed package failed its last verification and was discarded.
    pub(super) fn after_failed_install(version: String, error: &UpdateError) -> Self {
        Self::Failed {
            operation: FailedOperation::Install { version },
            message: error_chain(error),
        }
    }

    /// Starts installation only when the state owns the confirmed release and its artifact.
    ///
    /// A background check can replace the release between the confirmation and this call; the
    /// replacement stays ready but is never installed on a confirmation that was not for it.
    pub(super) fn begin_install(&mut self, version: &str) -> Result<ReadyUpdate<I>, UpdateError> {
        let previous = std::mem::replace(self, Self::Current);
        match previous {
            Self::Ready(ready) if ready.descriptor.identity.release_version != version => {
                *self = Self::Ready(ready);
                Err(UpdateError::ReleaseReplaced)
            }
            Self::Ready(ready) => {
                *self = Self::Installing(ready.clone());
                Ok(ready)
            }
            state => {
                *self = state;
                Err(UpdateError::NoPendingUpdate)
            }
        }
    }
}

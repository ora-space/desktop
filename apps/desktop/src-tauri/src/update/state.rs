//! Runtime state machine for one Desktop update operation.

use super::{DesktopUpdateStatus, ManualUpdateReason, ReleaseNote, UpdateError};
use crate::update::artifact_store::{ArtifactDescriptor, StoredArtifact};

/// Binds an installer handle to the exact verified artifact it is allowed to install.
#[derive(Clone)]
pub(super) struct ReadyUpdate<I> {
    pub(super) installer: I,
    pub(super) descriptor: ArtifactDescriptor,
    pub(super) artifact: StoredArtifact,
    pub(super) notes: Vec<ReleaseNote>,
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
            Self::Failed { message } => DesktopUpdateStatus::Failed {
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

    /// Starts installation only when the state owns the matching installer and artifact.
    pub(super) fn begin_install(&mut self) -> Result<ReadyUpdate<I>, UpdateError> {
        let previous = std::mem::replace(self, Self::Current);
        match previous {
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

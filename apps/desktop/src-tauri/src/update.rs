//! Desktop application update orchestration.
//!
//! The updater endpoint is owned by Tauri's signed updater plugin. This module only owns the
//! Desktop lifecycle around it: scheduling checks, persisting verified release artifacts, and
//! exposing a small status surface to the main webview.

mod artifact_store;
pub mod commands;
mod engine;
#[cfg(test)]
mod engine_tests;
mod job;
mod platform;
mod release_notes;
mod service;
mod state;
#[cfg(test)]
mod tests;
mod verifier;

pub use release_notes::ReleaseNote;
pub use service::{DesktopUpdateMode, UpdateService};

use ora_scheduler::SchedulerError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Explains why the running installation cannot replace itself with a downloaded package.
///
/// The Tauri updater dispatches on the bundle type baked into the binary, so a package that was
/// installed by a system package manager would be handed an artifact its installer cannot read.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManualUpdateReason {
    /// Installed from a distribution package that only its own package manager may replace.
    SystemPackage,
    /// Running as a bare executable, so there is no AppImage for the updater to swap.
    UnpackagedBinary,
}

/// Reports the lifecycle state consumed by the Desktop webview.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopUpdateStatus {
    /// No newer signed update is currently available.
    Current,
    /// A static updater manifest is being checked.
    Checking,
    /// The signed updater package is being downloaded.
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    /// A verified package is ready for an explicit user installation.
    Ready {
        version: String,
        /// Untrusted manifest notes, empty when the manifest has none that may be shown.
        notes: Vec<ReleaseNote>,
    },
    /// A newer release exists but this installation must be updated by other means.
    ManualUpdate {
        version: String,
        reason: ManualUpdateReason,
        /// Untrusted manifest notes, empty when the manifest has none that may be shown.
        notes: Vec<ReleaseNote>,
    },
    /// The update installation has started.
    Installing { version: String },
    /// The last check or installation failed; the next scheduled check may retry.
    Failed {
        /// Which operation failed, so the webview does not infer it from its own history.
        #[serde(flatten)]
        operation: FailedOperation,
        message: String,
    },
}

/// Names the operation behind a failed status, and for an installation the release it was for.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum FailedOperation {
    /// Checking the manifest, downloading, or verifying a package before it is offered.
    Check,
    /// Re-verifying a confirmed package right before it is handed to the installer.
    Install { version: String },
}

/// Reports failures from cache preparation, updater requests, or installation.
#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("failed to create update cache directory")]
    CacheDirectory(#[source] std::io::Error),
    #[error("failed to inspect update cache")]
    CacheInspect(#[source] std::io::Error),
    #[error("failed to read update cache metadata")]
    ReadMetadata(#[source] std::io::Error),
    #[error("failed to encode update cache metadata")]
    EncodeMetadata(#[source] serde_json::Error),
    #[error("failed to write update cache")]
    CacheWrite(#[source] std::io::Error),
    #[error("failed to commit update cache")]
    CacheCommit(#[source] std::io::Error),
    #[error("failed to read cached update")]
    CacheRead(#[source] std::io::Error),
    #[error("cached update digest changed")]
    CachedArtifactChanged,
    #[error("cached update signature is invalid")]
    CachedArtifactUntrusted,
    #[error("the updater trust configuration is invalid: {0}")]
    TrustConfiguration(String),
    #[error("update operation failed")]
    Updater(#[source] tauri_plugin_updater::Error),
    #[error("failed to read configured proxy settings. Review Settings → Network Proxy: {0}")]
    ProxySettings(String),
    #[error(transparent)]
    Proxy(#[from] ora_utils::http::HostProxyError),
    #[error("no update is ready to install")]
    NoPendingUpdate,
    #[error("the confirmed release was replaced by another one")]
    ReleaseReplaced,
    #[error("failed to register update schedule")]
    Scheduler(#[source] SchedulerError),
}

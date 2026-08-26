use std::path::PathBuf;

use gitlancer::GitlancerError;
use ora_plugin_manifest::UrlError;
use ora_utils::GitBranchNameError;
use thiserror::Error;

/// Reports failures produced while syncing a registry source or reading/writing an index.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Wraps a Git sync failure so marketplace refresh surfaces the underlying git error.
    #[error("registry source sync failed: {0}")]
    Git(#[from] GitlancerError),

    /// Wraps a manifest parse failure so a single malformed file can be diagnosed.
    #[error("plugin manifest parse failed: {0}")]
    Manifest(#[from] ora_plugin_manifest::ManifestError),

    /// Wraps an invalid marketplace source URL before a checkout directory is derived.
    #[error("marketplace source URL invalid: {0}")]
    SourceUrl(#[from] UrlError),

    /// Wraps an invalid marketplace source branch before any Git operation runs.
    #[error("marketplace source branch invalid: {0}")]
    SourceBranch(#[from] GitBranchNameError),

    /// Wraps a filesystem failure while scanning the registry tree or reading/writing files.
    #[error("registry file operation failed: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps an index serialization or deserialization failure.
    #[error("registry index JSON failed: {0}")]
    Json(#[from] serde_json::Error),

    /// Returned when a sync target has no usable parent directory to clone into.
    #[error("registry source clone destination has no parent directory: {0}")]
    MissingCloneParent(PathBuf),
}

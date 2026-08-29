//! Projects plugin package install, import, and update failures onto public backend errors.
//!
//! Host incompatibility is the only package-install failure that may leave the backend as a
//! typed public error. Other install and update variants stay internal so absolute package paths,
//! archive contents, and manifest text cannot reach the desktop adapter.

use crate::error::{BackendError, ErrorClassification};
use ora_contracts::{PluginHostVersionIncompatibleParams, PublicError};
use ora_plugin_manager::{InstallError, UpdateError};

/// Maps an install or local-import failure, keeping host incompatibility as a bounded public error.
pub(crate) fn map_install_error(error: InstallError) -> BackendError {
    match error {
        InstallError::HostVersionIncompatible(incompatibility) => BackendError::new(
            ErrorClassification::Unprocessable,
            PublicError::PluginHostVersionIncompatible(PluginHostVersionIncompatibleParams {
                actual_host_version: incompatibility.actual_host_version(),
                required_version_constraint: incompatibility.required_version_constraint(),
            }),
            "plugin requires an incompatible Ora Desktop version",
        ),
        InstallError::Download(_)
        | InstallError::MissingRelease
        | InstallError::Extract { .. }
        | InstallError::MissingManifest
        | InstallError::InvalidManifest(_)
        | InstallError::InvalidPackage { .. }
        | InstallError::ChecksumMismatch { .. }
        | InstallError::AlreadyInstalled { .. }
        | InstallError::Io { .. } => BackendError::internal("failed to install plugin", error),
    }
}

/// Maps an update failure through the same install projection so host incompatibility stays public.
pub(crate) fn map_update_error(error: UpdateError) -> BackendError {
    match error {
        UpdateError::Install(install_error) => map_install_error(install_error),
        UpdateError::NotFound { .. }
        | UpdateError::AlreadyUpToDate { .. }
        | UpdateError::Downgrade { .. }
        | UpdateError::Retire { .. } => BackendError::internal("failed to update plugin", error),
    }
}

//! Pack installation result types: per-member outcomes, failures, and rollback diagnostics
//! carried by [`InstallOutcome::PackInstalled`](crate::InstallOutcome).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies the first pack member whose installation failed and the classified reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PackInstallFailure {
    pub plugin_id: String,
    /// The stable public error code the member's install failure classified as.
    pub error_code: String,
    /// Members created before the original failure whose rollback also failed, in creation
    /// order. These members remain installed and are journaled as pack-managed so a later
    /// uninstall or retry can recover. Empty when the rollback completed.
    pub rollback_failures: Vec<PackRollbackFailure>,
}

/// One member whose rollback failed during a failed pack install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PackRollbackFailure {
    pub plugin_id: String,
    /// The stable public error code the member's rollback failure classified as.
    pub error_code: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PackInstallFailure::export(config)?;
    PackRollbackFailure::export(config)?;
    Ok(())
}

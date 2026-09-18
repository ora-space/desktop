//! Per-document workflow import outcomes for a local `.orax` package.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes what happened to one workflow document an imported `.orax` package carried.
///
/// Each document is imported on its own, so one unparseable or unrunnable document never costs
/// the user the working workflows beside it. The outcome carries the document's package-relative
/// path in both arms so a caller can report a failure against the exact file that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum ImportedWorkflowOutcome {
    /// The document became a workflow with one published snapshot.
    Imported {
        /// Package-relative path the document was read from, e.g. `assets/workflows/1.0.0.json`.
        source_file: String,
        /// Identifier of the created workflow.
        workflow_id: String,
        /// Workflow name taken from the document.
        name: String,
        /// The version the new snapshot was published under.
        version: String,
    },
    /// The document was refused and no workflow was created for it.
    Failed {
        /// Package-relative path the document was read from, e.g. `assets/workflows/1.0.0.json`.
        source_file: String,
        /// Human-readable reason the document could not become a workflow.
        reason: String,
    },
}

/// Exports the payload while the import flow and its workflow library stay host-side.
pub(super) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ImportedWorkflowOutcome::export(config)
}

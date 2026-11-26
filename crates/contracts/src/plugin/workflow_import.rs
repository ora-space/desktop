//! Per-document workflow import outcomes for a local `.orax` package and for the marketplace.

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

/// Describes what happened to one workflow document a marketplace install, update, or retry
/// considered.
///
/// The marketplace imports documents by the same rules as a local `.orax` package, but it also
/// keeps a durable record per source path, and what that record means depends on the current copy
/// and the operation: an install opens a new cycle and may recover a copy the user deleted, while
/// an update or a retry keeps respecting that deletion. A surviving copy stays skipped, a name
/// conflict stays retryable by fixing the name and asking again, and only a genuine failure keeps
/// no record at all. Every arm names the member plugin that carried the document, because one pack
/// install can carry documents from several members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum MarketplaceWorkflowOutcome {
    /// The document became a workflow with one published snapshot, and its copy is recorded.
    Imported {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path the document was read from, e.g. `assets/workflows/1.0.0.json`.
        source_file: String,
        /// Identifier of the created workflow.
        workflow_id: String,
        /// Workflow name taken from the document.
        name: String,
        /// The version the new snapshot was published under.
        version: String,
    },
    /// The document already has a surviving copy, so it was not imported again.
    ///
    /// This covers every path an earlier install already delivered, including a copy the user has
    /// renamed or edited since. Reporting it explains why a document shipped by the package did not
    /// appear in the library a second time.
    SkippedProcessed {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path whose copy is still alive.
        source_file: String,
    },
    /// The document's copy was deleted during the current install cycle, so it was left deleted.
    ///
    /// An install opens a new cycle and reconsiders the path; an upgrade or an explicit retry keeps
    /// respecting the deletion. This arm stays separate from
    /// [`MarketplaceWorkflowOutcome::SkippedProcessed`] so the surface can present the deletion as a
    /// neutral fact with its recovery path instead of as "already handled".
    SkippedDeletedCopy {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path whose copy the user deleted.
        source_file: String,
    },
    /// A workflow with this document's name already existed; the user's copy was kept.
    ///
    /// The conflict stays reported until the name is free again: the source path keeps a
    /// `conflict_skipped` record, and every later install, upgrade, or retry re-checks the library,
    /// so renaming or deleting the user's workflow makes the document importable. `known_conflict`
    /// marks a conflict the record already carried before this operation, which lets an upgrade
    /// report it as a neutral count instead of opening the report again.
    SkippedConflict {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path whose document name collided with an existing workflow.
        source_file: String,
        /// Name of the surviving workflow in the library that blocks this document's import.
        name: String,
        /// Whether the record already held this conflict before the operation ran.
        known_conflict: bool,
    },
    /// The document was refused and no record was created, so a retry can try it again.
    Failed {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path the document was read from, e.g. `assets/workflows/1.0.0.json`.
        source_file: String,
        /// Human-readable reason the document could not become a workflow.
        reason: String,
    },
}

/// Requests a retry of the workflow document import for one installed plugin.
///
/// The retry is an explicit action on an installed plugin card rather than a second install, so it
/// never downloads or replaces a package; it reconsiders paths without a surviving copy, including
/// unrecorded documents and name conflicts, while keeping copies deleted during the running cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct RetryWorkflowImportRequest {
    /// Canonical identifier of the installed plugin whose documents are retried.
    pub plugin_id: String,
}

/// Reports the per-document outcomes of one explicit workflow document retry.
///
/// The shape mirrors the install and update responses so the surface can present a retry summary
/// with the same success, skip, and failure vocabulary instead of inventing a second one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct RetryWorkflowImportResponse {
    /// Canonical identifier of the plugin whose documents were retried.
    pub plugin_id: String,
    /// One entry per document the retry considered, in package order.
    pub workflows: Vec<MarketplaceWorkflowOutcome>,
}

/// Exports the payload while the import flow and its workflow library stay host-side.
pub(super) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ImportedWorkflowOutcome::export(config)?;
    MarketplaceWorkflowOutcome::export(config)?;
    RetryWorkflowImportRequest::export(config)?;
    RetryWorkflowImportResponse::export(config)
}

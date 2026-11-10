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
/// keeps a durable record per source path. A path that already reached a terminal state is skipped
/// without touching the user's copy, the first name conflict is reported once and then remembered
/// as terminal, and only a genuine failure stays retryable. Every arm names the member plugin that
/// carried the document, because one pack install can carry documents from several members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum MarketplaceWorkflowOutcome {
    /// The document became a workflow with one published snapshot, and the source path is now
    /// terminal.
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
    /// The source path already had a terminal record, so the document was not imported again.
    ///
    /// This is the outcome of every path an earlier install, upgrade, or conflict skip already
    /// handled. Reporting it explains why a document shipped by the package did not appear in the
    /// library a second time.
    SkippedProcessed {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path that already reached a terminal state.
        source_file: String,
    },
    /// A workflow with this document's name already existed; the user's copy was kept.
    ///
    /// The conflict is reported only the first time: the source path becomes terminal, so later
    /// upgrades and retries report [`MarketplaceWorkflowOutcome::SkippedProcessed`] instead of the
    /// same conflict again.
    SkippedConflict {
        /// Canonical identifier of the member plugin that carried the document.
        plugin_id: String,
        /// Package-relative path whose document name collided with an existing workflow.
        source_file: String,
    },
    /// The document was refused and no terminal record was written, so a retry can try it again.
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
/// never downloads or replaces a package; it reconsiders only the documents whose source paths do
/// not have a terminal record yet.
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

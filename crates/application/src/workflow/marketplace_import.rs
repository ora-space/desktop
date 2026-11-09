//! Imports the workflow documents a marketplace plugin delivers, under durable source records.
//!
//! The marketplace differs from a local `.orax` import in one way that shapes this whole module:
//! an install and every later upgrade see the same package paths again. The import therefore
//! needs a durable, terminal record per source path, so a path is never imported twice and a
//! conflict with the user's own workflow is reported once instead of on every upgrade. See the
//! `20261109-marketplace-workflow-import` decision for the rules this implements.

use std::sync::Arc;

use ora_contracts::MarketplaceWorkflowOutcome;
use ora_domain::{AuditFields, Namespace, Workflow, WorkflowSnapshot};

use crate::RepositoryError;
use crate::workflow::import::{derive_publish_version, parse_workflow_document};
use crate::workflow::ports::{WorkflowIdGenerator, WorkflowRepository};
use crate::workflow::version::DRAFT_VERSION;
use crate::{Clock, workflow::version::is_publishable_version};

/// Identifies one workflow document inside one marketplace package.
///
/// The three fields are the whole identity of a source path: the complete canonical plugin id of
/// the member that delivered the document, plus the package-relative path. Package version,
/// document title and the user's current workflow name deliberately take no part, so renaming or
/// upgrading the package cannot make the same path look new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDocumentSource {
    /// Namespace of the marketplace source that published the delivering plugin.
    pub namespace: String,
    /// Plugin identifier inside that namespace, e.g. `ora.cdase-workflow`.
    pub identifier: String,
    /// Package-relative path, e.g. `assets/workflows/1.0.0.json`.
    pub document_path: String,
}

impl WorkflowDocumentSource {
    /// Returns the canonical `<namespace>/<identifier>` id the surface reports the outcome under.
    pub fn plugin_id(&self) -> String {
        format!("{}/{}", self.namespace, self.identifier)
    }
}

/// The terminal states a source path can reach.
///
/// A path in one of these states is finished forever: the marketplace never imports it again,
/// even after the package is upgraded, reinstalled, or the user deletes the copy it produced.
/// A failure is deliberately not representable here, which is what keeps failed documents
/// retryable without a second "failed" record to clean up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDocumentSourceState {
    /// The document became a workflow with one published snapshot.
    Imported,
    /// A workflow of the same name already existed; the user's copy was kept and the conflict must
    /// never be reported again.
    ConflictSkipped,
}

/// A workflow, its draft, its first published snapshot, and the source record that must commit
/// with them.
///
/// `commit_import` receives all four so the adapter can write them in one transaction. That is the
/// consistency guarantee the decision asks for: no observer may see a copy without its record, or
/// a record without its copy and published snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImportedWorkflow {
    /// Source path the workflow is recorded under.
    pub source: WorkflowDocumentSource,
    /// The created workflow definition.
    pub workflow: Workflow,
    /// Its editable draft snapshot, holding the document verbatim.
    pub draft: WorkflowSnapshot,
    /// Its first published snapshot, holding the document verbatim.
    pub published: WorkflowSnapshot,
}

/// Persists terminal marketplace workflow document outcomes.
///
/// The record outlives the copy it describes: it must survive uninstalling and reinstalling the
/// package, and deleting the workflow itself, because it is the only thing that stops an upgrade
/// from bringing a deleted workflow back. Implementations therefore store it independently of the
/// workflow aggregate and never cascade a software delete from it.
pub trait WorkflowDocumentSourceRepository {
    /// Returns the terminal state recorded for one source path, if any.
    fn find_state(
        &self,
        source: &WorkflowDocumentSource,
    ) -> Result<Option<WorkflowDocumentSourceState>, RepositoryError>;

    /// Records the first name conflict for one source path so later upgrades stay quiet.
    fn record_conflict_skip(
        &self,
        source: &WorkflowDocumentSource,
        recorded_at: i64,
    ) -> Result<(), RepositoryError>;

    /// Commits the created workflow, its draft, its published snapshot, and the `imported` record
    /// together, so a partial state can never be observed by a later install or retry.
    fn commit_import(&self, imported: NewImportedWorkflow) -> Result<(), RepositoryError>;
}

/// One workflow document a marketplace package contributed, attributed to its member plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceWorkflowDocument {
    /// Source path the document is recorded under.
    pub source: WorkflowDocumentSource,
    /// The document's text, taken from the package without rewriting it.
    pub contents: String,
}

/// Imports marketplace workflow documents under their durable source records.
///
/// It reuses the local import's document rules (JSON, workflow name, executable graph, version
/// derivation) so a document the marketplace refuses fails for the same reason it would fail when
/// dragged in from a local package. What it adds is the record: a terminal path is skipped, a
/// first name conflict is recorded and reported once, and only a genuine failure stays retryable.
pub struct ImportMarketplaceWorkflowsHandler<WorkflowStore, SourceStore, IdGenerator, ClockSource> {
    workflows: Arc<WorkflowStore>,
    sources: SourceStore,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<WorkflowStore, SourceStore, IdGenerator, ClockSource>
    ImportMarketplaceWorkflowsHandler<WorkflowStore, SourceStore, IdGenerator, ClockSource>
{
    /// Builds the marketplace import use case from the library, the record store, and its identity
    /// and time sources.
    pub fn new(
        workflows: Arc<WorkflowStore>,
        sources: SourceStore,
        id_generator: IdGenerator,
        clock: ClockSource,
    ) -> Self {
        Self {
            workflows,
            sources,
            id_generator,
            clock,
        }
    }
}

impl<WorkflowStore, SourceStore, IdGenerator, ClockSource>
    ImportMarketplaceWorkflowsHandler<WorkflowStore, SourceStore, IdGenerator, ClockSource>
where
    WorkflowStore: WorkflowRepository + Send + Sync + 'static,
    SourceStore: WorkflowDocumentSourceRepository,
    IdGenerator: WorkflowIdGenerator,
    ClockSource: Clock,
{
    /// Imports every document, returning one outcome per document in the order given.
    ///
    /// Documents are independent: one refused document reports as that document's outcome and
    /// never aborts the batch or rolls back the documents beside it.
    pub fn handle(
        &self,
        documents: Vec<MarketplaceWorkflowDocument>,
    ) -> Vec<MarketplaceWorkflowOutcome> {
        documents
            .into_iter()
            .map(|document| self.import_one(document))
            .collect()
    }

    /// Imports one document, turning every failure into that document's own outcome.
    fn import_one(&self, document: MarketplaceWorkflowDocument) -> MarketplaceWorkflowOutcome {
        let plugin_id = document.source.plugin_id();
        let source_file = document.source.document_path.clone();
        match self.sources.find_state(&document.source) {
            Ok(Some(_)) => {
                return MarketplaceWorkflowOutcome::SkippedProcessed {
                    plugin_id,
                    source_file,
                };
            }
            Ok(None) => {}
            Err(error) => {
                return MarketplaceWorkflowOutcome::Failed {
                    plugin_id,
                    source_file,
                    reason: error.to_string(),
                };
            }
        }
        match self.try_import(&document) {
            Ok(outcome) => outcome,
            Err(reason) => MarketplaceWorkflowOutcome::Failed {
                plugin_id: document.source.plugin_id(),
                source_file: document.source.document_path,
                reason,
            },
        }
    }

    /// Creates the workflow one document describes and records its source path.
    ///
    /// A name the library already uses is a conflict, not a failure: the user's workflow keeps its
    /// name and the source path becomes terminal so the same conflict is reported once. Every other
    /// error leaves no record, so a retry can try the path again after the cause is fixed.
    fn try_import(
        &self,
        document: &MarketplaceWorkflowDocument,
    ) -> Result<MarketplaceWorkflowOutcome, String> {
        let parsed = parse_workflow_document(&document.contents)?;
        let namespace = Namespace::local();
        let plugin_id = document.source.plugin_id();
        let source_file = document.source.document_path.clone();
        let already_taken = self
            .workflows
            .find_workflow_by_name(&namespace, &parsed.name)
            .map_err(|error| error.to_string())?
            .is_some();
        if already_taken {
            self.sources
                .record_conflict_skip(&document.source, self.clock.now_timestamp_millis())
                .map_err(|error| error.to_string())?;
            return Ok(MarketplaceWorkflowOutcome::SkippedConflict {
                plugin_id,
                source_file,
            });
        }

        let now = self.clock.now_timestamp_millis();
        let workflow_id = self.id_generator.generate_workflow_id();
        let workflow = Workflow::new(
            workflow_id.clone(),
            namespace,
            parsed.name.clone(),
            /*published_snapshot_id*/ None,
            AuditFields::new(now, now, /*is_deleted*/ false),
        )
        .map_err(|error| error.to_string())?;
        let draft = WorkflowSnapshot::new(
            self.id_generator.generate_snapshot_id(),
            workflow_id.clone(),
            DRAFT_VERSION,
            document.contents.clone(),
            now,
            Some(now),
            /*is_deleted*/ false,
        );
        // The published version follows the same precedence as a local import, so the same package
        // delivers the same version whichever entry point imported it.
        let version = derive_publish_version(
            parsed.version.as_deref(),
            &document.source.document_path,
            &parsed.name,
        )
        .unwrap_or_else(|| automatic_version(now));
        let published = WorkflowSnapshot::new(
            self.id_generator.generate_snapshot_id(),
            workflow_id.clone(),
            version.clone(),
            document.contents.clone(),
            now,
            Some(now),
            /*is_deleted*/ false,
        );
        self.sources
            .commit_import(NewImportedWorkflow {
                source: document.source.clone(),
                workflow,
                draft,
                published,
            })
            .map_err(|error| error.to_string())?;

        Ok(MarketplaceWorkflowOutcome::Imported {
            plugin_id,
            source_file,
            workflow_id: workflow_id.to_string(),
            name: parsed.name,
            version,
        })
    }
}

/// Returns the automatic publish version for a document that names none.
///
/// Import is a single-document commit rather than the publish handler's collision retry loop, so an
/// unusable derived version falls back to the same timestamp spelling the handler mints.
fn automatic_version(now: i64) -> String {
    let version = format!("v{now}");
    debug_assert!(is_publishable_version(&version));
    version
}

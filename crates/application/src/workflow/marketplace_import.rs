//! Imports the workflow documents a marketplace plugin delivers, under durable source records.
//!
//! The marketplace differs from a local `.orax` import in one way that shapes this whole module:
//! an install and every later upgrade see the same package paths again. The import therefore
//! needs a durable record per source path, and what that record allows depends on the current copy
//! and on the operation that is asking. A successful install opens an install cycle and may recover
//! a copy the user deleted; an upgrade and the explicit card retry stay inside the cycle and keep
//! respecting that deletion, while a name conflict stays retryable by freeing the name. See the
//! `20261124-marketplace-workflow-conflict-recovery` decision for the rules this implements.

use std::sync::Arc;

use ora_contracts::MarketplaceWorkflowOutcome;
use ora_domain::{AuditFields, Namespace, Workflow, WorkflowSnapshot};

use crate::RepositoryError;
use crate::workflow::import::{derive_publish_version, parse_workflow_document};
use crate::workflow::ports::{WorkflowIdGenerator, WorkflowRepository};
use crate::workflow::version::DRAFT_VERSION;
use crate::{Clock, workflow::version::is_publishable_version};

/// How many times one document re-reads its source record before it reports a failure.
///
/// A conditional write that finds another operation already moved the record is not an error: the
/// document is judged again against what that operation left behind. The bound only exists so a
/// pathological writer loop cannot spin forever.
const SOURCE_REREAD_LIMIT: u8 = 3;

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

/// Which marketplace operation is asking the import to consider the documents.
///
/// The operation decides whether a copy the user deleted may come back; it never decides whether a
/// conflict is reported, because that is read from the record the operation found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketplaceImportOperation {
    /// A successful marketplace install opens a new install cycle, so a copy deleted in an earlier
    /// cycle is recoverable while a copy deleted inside the running cycle is not.
    Install,
    /// An automatic upgrade stays inside the current cycle and keeps respecting every deletion the
    /// cycle saw, while still re-judging unrecorded paths and name conflicts.
    Update,
    /// The explicit retry on the installed card stays inside the current cycle on the same terms as
    /// an upgrade.
    Retry,
}

/// What one source path's durable record says, together with the copy it points at.
///
/// Liveness is resolved by the store on every read rather than stored, because the record has to
/// stay valid while the user deletes or restores the copy beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDocumentSourceRecord {
    /// The path produced a copy the user still has, possibly renamed or edited since.
    ImportedAlive,
    /// The path produced a copy the user has since deleted.
    ImportedDeleted,
    /// The path collided with a workflow the user already had, so nothing was created.
    ConflictSkipped,
}

/// The record state an import read before writing, so the write can be made conditional on it.
///
/// Carrying the expectation into the store is what keeps two operations from both claiming one
/// source path: a write whose expectation no longer holds is refused instead of overwriting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePrecondition {
    /// The path had no record at all.
    Unrecorded,
    /// The record pointed at a copy the user deleted, and the path is being recovered.
    DeletedCopy,
    /// The record held a name conflict that is no longer present in the library.
    ConflictSkipped,
}

/// The result of committing one import against a source precondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportCommit {
    /// The copy, its two snapshots, and the `imported` record are committed together.
    Committed,
    /// Another write occupied the document's name first, so this copy was rolled back.
    NameTaken,
    /// Another write changed the source record first, so this copy was rolled back and the caller
    /// has to judge the document again against the state that write left behind.
    SourceChanged,
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

/// Persists marketplace workflow document records and the copies they describe.
///
/// The record outlives the copy it describes: it must survive uninstalling and reinstalling the
/// package, and deleting the workflow itself, because it is what tells a later operation whether
/// the path already delivered a copy. Implementations therefore store it independently of the
/// workflow aggregate and never cascade a software delete from it.
///
/// Every write here is conditional: the caller passes the state it read, and a write that no longer
/// matches is refused rather than applied. That keeps a losing writer from overwriting the winner
/// and from reporting a plain conflict as a failure.
pub trait WorkflowDocumentSourceRepository {
    /// Returns the record for one source path and whether the copy it points at still exists.
    fn find_record(
        &self,
        source: &WorkflowDocumentSource,
    ) -> Result<Option<WorkflowDocumentSourceRecord>, RepositoryError>;

    /// Records a name conflict for one source path, leaving an `imported` record untouched.
    fn record_conflict_skip(
        &self,
        source: &WorkflowDocumentSource,
        recorded_at: i64,
    ) -> Result<(), RepositoryError>;

    /// Commits the created workflow, its draft, its published snapshot, and the `imported` record
    /// together, conditional on the source state the caller read and on the document's name still
    /// being free.
    fn commit_import(
        &self,
        imported: NewImportedWorkflow,
        precondition: SourcePrecondition,
    ) -> Result<ImportCommit, RepositoryError>;

    /// Clears an `imported` record whose copy the user deleted, so the path is treated as one the
    /// package never delivered and the document becomes retryable again.
    ///
    /// Returns whether the record was cleared; `false` means the expectation no longer held.
    fn forget_deleted_copy(&self, source: &WorkflowDocumentSource)
    -> Result<bool, RepositoryError>;
}

/// One workflow document a marketplace package contributed, attributed to its member plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceWorkflowDocument {
    /// Source path the document is recorded under.
    pub source: WorkflowDocumentSource,
    /// The document's text, taken from the package without rewriting it.
    pub contents: String,
}

/// Whether one document's attempt settled on an outcome or has to be judged again.
enum DocumentAttempt {
    /// The document reached the outcome it reports.
    Resolved(MarketplaceWorkflowOutcome),
    /// Another operation moved the source record first, so the document is judged again.
    SourceChanged,
}

/// Imports marketplace workflow documents under their durable source records.
///
/// It reuses the local import's document rules (JSON, workflow name, executable graph, version
/// derivation) so a document the marketplace refuses fails for the same reason it would fail when
/// dragged in from a local package. What it adds is the record and the cycle: a surviving copy is
/// skipped, a deleted copy is recovered only by a fresh install, a name conflict is reported until
/// the name is free, and only a genuine failure leaves the path retryable.
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
    /// never aborts the batch or rolls back the documents beside it. The operation decides whether
    /// this run may recover copies deleted in an earlier install cycle.
    pub fn handle(
        &self,
        operation: MarketplaceImportOperation,
        documents: Vec<MarketplaceWorkflowDocument>,
    ) -> Vec<MarketplaceWorkflowOutcome> {
        documents
            .into_iter()
            .map(|document| self.import_one(operation, document))
            .collect()
    }

    /// Judges one document against its record and imports it when the record allows that.
    fn import_one(
        &self,
        operation: MarketplaceImportOperation,
        document: MarketplaceWorkflowDocument,
    ) -> MarketplaceWorkflowOutcome {
        let mut rereads = 0;
        loop {
            let attempt = match self.sources.find_record(&document.source) {
                Ok(None) => self.import_document(&document, SourcePrecondition::Unrecorded, false),
                Ok(Some(WorkflowDocumentSourceRecord::ConflictSkipped)) => self.import_document(
                    &document,
                    SourcePrecondition::ConflictSkipped,
                    /*known_conflict*/ true,
                ),
                Ok(Some(WorkflowDocumentSourceRecord::ImportedAlive)) => {
                    DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::SkippedProcessed {
                        plugin_id: document.source.plugin_id(),
                        source_file: document.source.document_path.clone(),
                    })
                }
                // Only a new install cycle may reconsider a copy the user deleted; an upgrade and
                // the card retry stay inside the cycle and keep the deletion.
                Ok(Some(WorkflowDocumentSourceRecord::ImportedDeleted))
                    if operation == MarketplaceImportOperation::Install =>
                {
                    self.import_document(&document, SourcePrecondition::DeletedCopy, false)
                }
                Ok(Some(WorkflowDocumentSourceRecord::ImportedDeleted)) => {
                    DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::SkippedDeletedCopy {
                        plugin_id: document.source.plugin_id(),
                        source_file: document.source.document_path.clone(),
                    })
                }
                Err(error) => DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::Failed {
                    plugin_id: document.source.plugin_id(),
                    source_file: document.source.document_path.clone(),
                    reason: error.to_string(),
                }),
            };
            match attempt {
                DocumentAttempt::Resolved(outcome) => return outcome,
                DocumentAttempt::SourceChanged if rereads < SOURCE_REREAD_LIMIT => rereads += 1,
                DocumentAttempt::SourceChanged => {
                    return MarketplaceWorkflowOutcome::Failed {
                        plugin_id: document.source.plugin_id(),
                        source_file: document.source.document_path,
                        reason: "the workflow document source kept changing while it was imported"
                            .to_string(),
                    };
                }
            }
        }
    }

    /// Creates the workflow one document describes and records its source path.
    ///
    /// A name the library already uses is a conflict, not a failure: the user's workflow keeps its
    /// name and the path keeps a `conflict_skipped` record that a later operation re-checks. Every
    /// other error leaves no record, so a retry can try the path again after the cause is fixed,
    /// and a failed recovery of a deleted copy returns the path to that state.
    fn import_document(
        &self,
        document: &MarketplaceWorkflowDocument,
        precondition: SourcePrecondition,
        known_conflict: bool,
    ) -> DocumentAttempt {
        let parsed = match parse_workflow_document(&document.contents) {
            Ok(parsed) => parsed,
            Err(reason) => return self.attempt_failed(document, precondition, reason),
        };
        let namespace = Namespace::local();
        let name_taken = match self
            .workflows
            .find_workflow_by_name(&namespace, &parsed.name)
        {
            Ok(found) => found.is_some(),
            Err(error) => {
                return self.attempt_failed(document, precondition, error.to_string());
            }
        };
        if name_taken {
            return self.attempt_conflict(document, &parsed.name, known_conflict);
        }

        let now = self.clock.now_timestamp_millis();
        let workflow_id = self.id_generator.generate_workflow_id();
        let workflow = match Workflow::new(
            workflow_id.clone(),
            namespace,
            parsed.name.clone(),
            /*published_snapshot_id*/ None,
            AuditFields::new(now, now, /*is_deleted*/ false),
        ) {
            Ok(workflow) => workflow,
            Err(error) => return self.attempt_failed(document, precondition, error.to_string()),
        };
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
        match self.sources.commit_import(
            NewImportedWorkflow {
                source: document.source.clone(),
                workflow,
                draft,
                published,
            },
            precondition,
        ) {
            Ok(ImportCommit::Committed) => {
                DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::Imported {
                    plugin_id: document.source.plugin_id(),
                    source_file: document.source.document_path.clone(),
                    workflow_id: workflow_id.to_string(),
                    name: parsed.name,
                    version,
                })
            }
            // The name was free when it was checked and taken before the commit landed: the copy
            // is rolled back and reported as the conflict it is, never as a failure.
            Ok(ImportCommit::NameTaken) => {
                self.attempt_conflict(document, &parsed.name, known_conflict)
            }
            // Another operation claimed the path first. The copy never lands, so judging the
            // document again against the record that operation left cannot create a second one.
            Ok(ImportCommit::SourceChanged) => DocumentAttempt::SourceChanged,
            Err(error) => self.attempt_failed(document, precondition, error.to_string()),
        }
    }

    /// Records the name conflict the document hit and reports it with the name it collided on.
    fn attempt_conflict(
        &self,
        document: &MarketplaceWorkflowDocument,
        name: &str,
        known_conflict: bool,
    ) -> DocumentAttempt {
        match self
            .sources
            .record_conflict_skip(&document.source, self.clock.now_timestamp_millis())
        {
            Ok(()) => DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::SkippedConflict {
                plugin_id: document.source.plugin_id(),
                source_file: document.source.document_path.clone(),
                name: name.to_string(),
                known_conflict,
            }),
            Err(error) => DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::Failed {
                plugin_id: document.source.plugin_id(),
                source_file: document.source.document_path.clone(),
                reason: error.to_string(),
            }),
        }
    }

    /// Reports a refusal, first handing a deleted copy's path back so the rest of the cycle can
    /// retry instead of waiting for another uninstall and install.
    ///
    /// A failure while clearing the record is deliberately not turned into a second outcome: the
    /// refusal itself is what the user has to fix, and keeping the record leaves the copy on the
    /// fallback path, where fixing the cause and reinstalling recovers it.
    fn attempt_failed(
        &self,
        document: &MarketplaceWorkflowDocument,
        precondition: SourcePrecondition,
        reason: String,
    ) -> DocumentAttempt {
        if precondition == SourcePrecondition::DeletedCopy {
            let _cleared = self.sources.forget_deleted_copy(&document.source);
        }
        DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::Failed {
            plugin_id: document.source.plugin_id(),
            source_file: document.source.document_path.clone(),
            reason,
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

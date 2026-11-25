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
use ora_domain::{AuditFields, Namespace, Workflow, WorkflowId, WorkflowSnapshot};

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
    /// A successful marketplace install opens a new install cycle, so any `imported` record whose
    /// copy is no longer alive is reconsidered for recovery while surviving copies stay untouched.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowDocumentSourceRecord {
    /// The path produced a copy the user still has, possibly renamed or edited since.
    ImportedAlive,
    /// The path produced the identified copy, which the user has since deleted.
    ImportedDeleted(WorkflowId),
    /// The path collided with a workflow the user already had, so nothing was created.
    ConflictSkipped,
}

/// The record state an import read before writing, so the write can be made conditional on it.
///
/// Carrying the expectation into the store is what keeps two operations from both claiming one
/// source path: a write whose expectation no longer holds is refused instead of overwriting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourcePrecondition {
    /// The path had no record at all.
    Unrecorded,
    /// The record pointed at the identified copy the user deleted, and the path is being
    /// recovered.
    DeletedCopy(WorkflowId),
    /// The record held a name conflict; the operation re-judges the name against the current
    /// library, so the conflict may still block the import or may already be free.
    ConflictSkipped,
}

/// The result of recording a name conflict against a source precondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSkipCommit {
    /// The `conflict_skipped` record was written or refreshed under the precondition.
    Recorded,
    /// Another write changed the source record first, so the caller must re-judge the document
    /// against what that write left behind.
    SourceChanged,
}

/// The result of committing one import against a source precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportCommit {
    /// The copy, its two snapshots, and the `imported` record are committed together.
    Committed,
    /// Another write occupied the document's name first (carrying that surviving workflow's name),
    /// so this copy was rolled back.
    NameTaken { blocking_name: String },
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

    /// Records a name conflict for one source path conditional on the state the caller read.
    fn record_conflict_skip(
        &self,
        source: &WorkflowDocumentSource,
        precondition: &SourcePrecondition,
        recorded_at: i64,
    ) -> Result<ConflictSkipCommit, RepositoryError>;

    /// Commits the created workflow, its draft, its published snapshot, and the `imported` record
    /// together, conditional on the source state the caller read and on the document's name still
    /// being free.
    fn commit_import(
        &self,
        imported: NewImportedWorkflow,
        precondition: &SourcePrecondition,
    ) -> Result<ImportCommit, RepositoryError>;

    /// Clears an `imported` record whose identified copy the user deleted, so the path is treated
    /// as one the package never delivered and the document becomes retryable again.
    ///
    /// Returns whether the record was cleared; `false` means the expectation no longer held.
    fn forget_deleted_copy(
        &self,
        source: &WorkflowDocumentSource,
        expected_workflow_id: &WorkflowId,
    ) -> Result<bool, RepositoryError>;
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
                Ok(None) => self.import_document(&document, &SourcePrecondition::Unrecorded),
                Ok(Some(WorkflowDocumentSourceRecord::ConflictSkipped)) => {
                    self.import_document(&document, &SourcePrecondition::ConflictSkipped)
                }
                Ok(Some(WorkflowDocumentSourceRecord::ImportedAlive)) => {
                    DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::SkippedProcessed {
                        plugin_id: document.source.plugin_id(),
                        source_file: document.source.document_path.clone(),
                    })
                }
                // Only a new install cycle may reconsider a copy the user deleted; an upgrade and
                // the card retry stay inside the cycle and keep the deletion.
                Ok(Some(WorkflowDocumentSourceRecord::ImportedDeleted(workflow_id)))
                    if operation == MarketplaceImportOperation::Install =>
                {
                    self.import_document(&document, &SourcePrecondition::DeletedCopy(workflow_id))
                }
                Ok(Some(WorkflowDocumentSourceRecord::ImportedDeleted(_))) => {
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
        precondition: &SourcePrecondition,
    ) -> DocumentAttempt {
        let parsed = match parse_workflow_document(&document.contents) {
            Ok(parsed) => parsed,
            Err(reason) => return self.attempt_failed(document, precondition, reason),
        };
        let namespace = Namespace::local();
        let blocking_name = match self
            .workflows
            .find_workflow_by_name(&namespace, &parsed.name)
        {
            Ok(found) => found.map(|workflow| workflow.name),
            Err(error) => {
                return self.attempt_failed(document, precondition, error.to_string());
            }
        };
        if let Some(blocking_name) = blocking_name {
            return self.attempt_conflict(document, precondition, &blocking_name);
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
            Ok(ImportCommit::NameTaken { blocking_name }) => {
                self.attempt_conflict(document, precondition, &blocking_name)
            }
            // Another operation claimed the path first. The copy never lands, so judging the
            // document again against the record that operation left cannot create a second one.
            Ok(ImportCommit::SourceChanged) => DocumentAttempt::SourceChanged,
            Err(error) => self.attempt_failed(document, precondition, error.to_string()),
        }
    }

    /// Records the name conflict the document hit and reports it with the surviving workflow name
    /// that blocked it.
    fn attempt_conflict(
        &self,
        document: &MarketplaceWorkflowDocument,
        precondition: &SourcePrecondition,
        blocking_name: &str,
    ) -> DocumentAttempt {
        match self.sources.record_conflict_skip(
            &document.source,
            precondition,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(ConflictSkipCommit::Recorded) => {
                DocumentAttempt::Resolved(MarketplaceWorkflowOutcome::SkippedConflict {
                    plugin_id: document.source.plugin_id(),
                    source_file: document.source.document_path.clone(),
                    name: blocking_name.to_string(),
                    known_conflict: matches!(precondition, SourcePrecondition::ConflictSkipped),
                })
            }
            Ok(ConflictSkipCommit::SourceChanged) => DocumentAttempt::SourceChanged,
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
    /// A clear that finds the record already changed re-judges the document against what won the
    /// race. A failure while clearing the record is deliberately not turned into a second outcome:
    /// the refusal itself is what the user has to fix, and keeping the record leaves the copy on the
    /// fallback path, where fixing the cause and reinstalling recovers it.
    fn attempt_failed(
        &self,
        document: &MarketplaceWorkflowDocument,
        precondition: &SourcePrecondition,
        reason: String,
    ) -> DocumentAttempt {
        if let SourcePrecondition::DeletedCopy(expected_workflow_id) = precondition {
            // `false` means another write moved the record first, so the document is judged again
            // against what that write left behind; an error keeps the record on the fallback path.
            if matches!(
                self.sources
                    .forget_deleted_copy(&document.source, expected_workflow_id),
                Ok(false)
            ) {
                return DocumentAttempt::SourceChanged;
            }
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use ora_contracts::MarketplaceWorkflowOutcome;
    use ora_domain::{
        AuditFields, CreatedWorkflow, Namespace, Workflow, WorkflowDetail, WorkflowId,
        WorkflowSnapshot, WorkflowSnapshotId, WorkflowSummary, WorkflowVersion,
    };
    use pretty_assertions::assert_eq;

    use super::{
        ConflictSkipCommit, ImportCommit, ImportMarketplaceWorkflowsHandler,
        MarketplaceImportOperation, MarketplaceWorkflowDocument, NewImportedWorkflow,
        SourcePrecondition, WorkflowDocumentSource, WorkflowDocumentSourceRecord,
        WorkflowDocumentSourceRepository,
    };
    use crate::workflow::ports::{
        ActivateVersionResult, DeleteSnapshotResult, DeleteWorkflowResult, PublishSnapshotResult,
        RollbackDraftResult, UpdateDraftResult, UpdateWorkflowResult, WorkflowIdGenerator,
        WorkflowRepository,
    };
    use crate::{Clock, RepositoryError};

    const VALID_DOCUMENT_LOWER: &str = r#"{"name":"release flow","version":"1.0.0","viewport":{"x":0,"y":0,"zoom":1},"nodes":[{"id":"start","type":"workflow","position":{"x":0,"y":0},"data":{"kind":"start","title":"开始"}}],"edges":[]}"#;

    #[derive(Clone, Copy, Debug)]
    struct FixedClock(i64);

    impl Clock for FixedClock {
        fn now_timestamp_millis(&self) -> i64 {
            self.0
        }
    }

    #[derive(Debug, Default, Clone)]
    struct CountingIdGenerator {
        next: Arc<Mutex<u64>>,
    }

    impl WorkflowIdGenerator for CountingIdGenerator {
        fn generate_workflow_id(&self) -> WorkflowId {
            let mut next = self.next.lock().expect("id generator lock");
            *next += 1;
            WorkflowId::new(format!("workflow-{next}"))
        }

        fn generate_snapshot_id(&self) -> WorkflowSnapshotId {
            let mut next = self.next.lock().expect("id generator lock");
            *next += 1;
            WorkflowSnapshotId::new(format!("snapshot-{next}"))
        }
    }

    #[derive(Debug, Default)]
    struct FakeWorkflowRepository {
        existing_by_name: Option<Workflow>,
    }

    impl WorkflowRepository for FakeWorkflowRepository {
        fn create_workflow(
            &self,
            _workflow: Workflow,
            _draft: WorkflowSnapshot,
        ) -> Result<CreatedWorkflow, RepositoryError> {
            unreachable!("marketplace import commits through the source repository")
        }

        fn find_workflow(
            &self,
            _workflow_id: &WorkflowId,
        ) -> Result<Option<Workflow>, RepositoryError> {
            unreachable!("marketplace import never looks up a workflow by id")
        }

        fn find_workflow_by_name(
            &self,
            _namespace: &Namespace,
            _name: &str,
        ) -> Result<Option<Workflow>, RepositoryError> {
            Ok(self.existing_by_name.clone())
        }

        fn get_workflow_detail(
            &self,
            _workflow_id: &WorkflowId,
        ) -> Result<Option<WorkflowDetail>, RepositoryError> {
            unreachable!("marketplace import never loads workflow details")
        }

        fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError> {
            unreachable!("marketplace import never lists workflows")
        }

        fn update_workflow(
            &self,
            _workflow_id: &WorkflowId,
            _name: String,
            _updated_at: i64,
        ) -> Result<UpdateWorkflowResult, RepositoryError> {
            unreachable!("marketplace import never updates workflows")
        }

        fn soft_delete_workflow(
            &self,
            _workflow_id: &WorkflowId,
            _deleted_at: i64,
        ) -> Result<DeleteWorkflowResult, RepositoryError> {
            unreachable!("marketplace import never deletes workflows")
        }

        fn find_snapshot_by_version(
            &self,
            _workflow_id: &WorkflowId,
            _version: &str,
        ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
            unreachable!("marketplace import never loads snapshots")
        }

        fn find_snapshot_by_id(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
        ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
            unreachable!("marketplace import never loads snapshots by id")
        }

        fn find_snapshot_any_workflow(
            &self,
            _snapshot_id: &WorkflowSnapshotId,
        ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
            unreachable!("marketplace import never loads snapshots across workflows")
        }

        fn list_versions(
            &self,
            _workflow_id: &WorkflowId,
        ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
            unreachable!("marketplace import never lists versions")
        }

        fn update_draft(
            &self,
            _workflow_id: &WorkflowId,
            _graph: String,
            _updated_at: i64,
        ) -> Result<UpdateDraftResult, RepositoryError> {
            unreachable!("marketplace import never edits drafts")
        }

        fn publish_snapshot(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: WorkflowSnapshotId,
            _version: String,
            _created_at: i64,
        ) -> Result<PublishSnapshotResult, RepositoryError> {
            unreachable!("marketplace import never publishes drafts")
        }

        fn rollback_draft(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
            _updated_at: i64,
        ) -> Result<RollbackDraftResult, RepositoryError> {
            unreachable!("marketplace import never rolls back drafts")
        }

        fn activate_version(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
            _updated_at: i64,
        ) -> Result<ActivateVersionResult, RepositoryError> {
            unreachable!("marketplace import never activates versions")
        }

        fn soft_delete_snapshot(
            &self,
            _workflow_id: &WorkflowId,
            _snapshot_id: &WorkflowSnapshotId,
            _deleted_at: i64,
        ) -> Result<DeleteSnapshotResult, RepositoryError> {
            unreachable!("marketplace import never deletes snapshots")
        }
    }

    #[derive(Debug, Default)]
    struct ScriptedSourceRepository {
        records: Mutex<VecDeque<Option<WorkflowDocumentSourceRecord>>>,
        commits: Mutex<VecDeque<ImportCommit>>,
        conflict_commits: Mutex<VecDeque<ConflictSkipCommit>>,
        recorded_conflict_preconditions: Mutex<Vec<SourcePrecondition>>,
        commit_count: Mutex<usize>,
        forgets: Mutex<VecDeque<Result<bool, RepositoryError>>>,
        forget_calls: Mutex<usize>,
    }

    impl WorkflowDocumentSourceRepository for &ScriptedSourceRepository {
        fn find_record(
            &self,
            _source: &WorkflowDocumentSource,
        ) -> Result<Option<WorkflowDocumentSourceRecord>, RepositoryError> {
            Ok(self
                .records
                .lock()
                .expect("records lock")
                .pop_front()
                .expect("scripted find_record result"))
        }

        fn record_conflict_skip(
            &self,
            _source: &WorkflowDocumentSource,
            precondition: &SourcePrecondition,
            _recorded_at: i64,
        ) -> Result<ConflictSkipCommit, RepositoryError> {
            self.recorded_conflict_preconditions
                .lock()
                .expect("preconditions lock")
                .push(precondition.clone());
            Ok(self
                .conflict_commits
                .lock()
                .expect("conflict_commits lock")
                .pop_front()
                .expect("scripted record_conflict_skip result"))
        }

        fn commit_import(
            &self,
            _imported: NewImportedWorkflow,
            _precondition: &SourcePrecondition,
        ) -> Result<ImportCommit, RepositoryError> {
            *self.commit_count.lock().expect("commit_count lock") += 1;
            Ok(self
                .commits
                .lock()
                .expect("commits lock")
                .pop_front()
                .expect("scripted commit_import result"))
        }

        fn forget_deleted_copy(
            &self,
            _source: &WorkflowDocumentSource,
            _expected_workflow_id: &WorkflowId,
        ) -> Result<bool, RepositoryError> {
            *self.forget_calls.lock().expect("forget_calls lock") += 1;
            self.forgets
                .lock()
                .expect("forgets lock")
                .pop_front()
                .unwrap_or(Ok(true))
        }
    }

    fn document(contents: &str) -> MarketplaceWorkflowDocument {
        MarketplaceWorkflowDocument {
            source: WorkflowDocumentSource {
                namespace: "official".to_string(),
                identifier: "ora.cdase-workflow".to_string(),
                document_path: "assets/workflows/1.0.0.json".to_string(),
            },
            contents: contents.to_string(),
        }
    }

    fn surviving_workflow(name: &str) -> Workflow {
        Workflow::new(
            WorkflowId::new("workflow-existing"),
            Namespace::local(),
            name,
            /*published_snapshot_id*/ None,
            AuditFields::new(1, 1, /*is_deleted*/ false),
        )
        .expect("valid workflow")
    }

    #[test]
    fn reports_a_commit_time_name_collision_as_skipped_conflict_with_the_blocking_name() {
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([
                None,
                Some(WorkflowDocumentSourceRecord::ConflictSkipped),
            ])),
            commits: Mutex::new(VecDeque::from([
                ImportCommit::NameTaken {
                    blocking_name: "Release Flow".to_string(),
                },
                ImportCommit::NameTaken {
                    blocking_name: "Release Flow".to_string(),
                },
            ])),
            conflict_commits: Mutex::new(VecDeque::from([
                ConflictSkipCommit::Recorded,
                ConflictSkipCommit::Recorded,
            ])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository::default()),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let first = handler.handle(
            MarketplaceImportOperation::Install,
            vec![document(VALID_DOCUMENT_LOWER)],
        );
        let second = handler.handle(
            MarketplaceImportOperation::Update,
            vec![document(VALID_DOCUMENT_LOWER)],
        );

        assert_eq!(
            first,
            vec![MarketplaceWorkflowOutcome::SkippedConflict {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
                name: "Release Flow".to_string(),
                known_conflict: false,
            }]
        );
        assert_eq!(
            second,
            vec![MarketplaceWorkflowOutcome::SkippedConflict {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
                name: "Release Flow".to_string(),
                known_conflict: true,
            }]
        );
        assert_eq!(
            *sources
                .recorded_conflict_preconditions
                .lock()
                .expect("preconditions lock"),
            vec![
                SourcePrecondition::Unrecorded,
                SourcePrecondition::ConflictSkipped,
            ]
        );
    }

    #[test]
    fn reports_the_blocking_workflow_name_for_a_precheck_case_variant() {
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([None])),
            conflict_commits: Mutex::new(VecDeque::from([ConflictSkipCommit::Recorded])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository {
                existing_by_name: Some(surviving_workflow("Release Flow")),
            }),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let outcomes = handler.handle(
            MarketplaceImportOperation::Install,
            vec![document(VALID_DOCUMENT_LOWER)],
        );

        assert_eq!(
            outcomes,
            vec![MarketplaceWorkflowOutcome::SkippedConflict {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
                name: "Release Flow".to_string(),
                known_conflict: false,
            }]
        );
    }

    #[test]
    fn rejudges_the_document_when_commit_reports_source_changed() {
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([
                None,
                Some(WorkflowDocumentSourceRecord::ImportedAlive),
            ])),
            commits: Mutex::new(VecDeque::from([ImportCommit::SourceChanged])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository::default()),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let outcomes = handler.handle(
            MarketplaceImportOperation::Install,
            vec![document(VALID_DOCUMENT_LOWER)],
        );

        assert_eq!(
            outcomes,
            vec![MarketplaceWorkflowOutcome::SkippedProcessed {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
            }]
        );
    }

    #[test]
    fn rejudges_the_document_when_record_conflict_skip_reports_source_changed() {
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([
                None,
                Some(WorkflowDocumentSourceRecord::ImportedDeleted(
                    WorkflowId::new("workflow-1"),
                )),
            ])),
            conflict_commits: Mutex::new(VecDeque::from([ConflictSkipCommit::SourceChanged])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository {
                existing_by_name: Some(surviving_workflow("Release Flow")),
            }),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let outcomes = handler.handle(
            MarketplaceImportOperation::Retry,
            vec![document(VALID_DOCUMENT_LOWER)],
        );

        assert_eq!(
            outcomes,
            vec![MarketplaceWorkflowOutcome::SkippedDeletedCopy {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
            }]
        );
    }

    #[test]
    fn reports_a_failure_when_source_keeps_changing_past_the_reread_limit() {
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([None, None, None, None])),
            commits: Mutex::new(VecDeque::from([
                ImportCommit::SourceChanged,
                ImportCommit::SourceChanged,
                ImportCommit::SourceChanged,
                ImportCommit::SourceChanged,
            ])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository::default()),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let outcomes = handler.handle(
            MarketplaceImportOperation::Install,
            vec![document(VALID_DOCUMENT_LOWER)],
        );

        assert_eq!(*sources.commit_count.lock().expect("commit_count lock"), 4);
        assert_eq!(
            outcomes,
            vec![MarketplaceWorkflowOutcome::Failed {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
                reason: "the workflow document source kept changing while it was imported"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn rejudges_a_failed_recovery_whose_deleted_copy_record_changed() {
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([
                Some(WorkflowDocumentSourceRecord::ImportedDeleted(
                    WorkflowId::new("workflow-1"),
                )),
                Some(WorkflowDocumentSourceRecord::ImportedAlive),
            ])),
            forgets: Mutex::new(VecDeque::from([Ok(false)])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository::default()),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let outcomes = handler.handle(
            MarketplaceImportOperation::Install,
            vec![document("not a workflow document")],
        );

        assert_eq!(
            outcomes,
            vec![MarketplaceWorkflowOutcome::SkippedProcessed {
                plugin_id: "official/ora.cdase-workflow".to_string(),
                source_file: "assets/workflows/1.0.0.json".to_string(),
            }],
            "a record another write moved is re-judged, not reported as the stale refusal"
        );
        assert_eq!(*sources.forget_calls.lock().expect("forget_calls lock"), 1);
    }

    #[test]
    fn keeps_the_record_on_the_fallback_path_when_clearing_it_fails() {
        // Exactly one scripted read: a re-judge here would pop an empty queue and panic, so the
        // test also proves the error path reports the refusal instead of looping.
        let sources = ScriptedSourceRepository {
            records: Mutex::new(VecDeque::from([Some(
                WorkflowDocumentSourceRecord::ImportedDeleted(WorkflowId::new("workflow-1")),
            )])),
            forgets: Mutex::new(VecDeque::from([Err(RepositoryError::from_message(
                "clear failed",
            ))])),
            ..Default::default()
        };
        let handler = ImportMarketplaceWorkflowsHandler::new(
            Arc::new(FakeWorkflowRepository::default()),
            &sources,
            CountingIdGenerator::default(),
            FixedClock(42),
        );

        let outcomes = handler.handle(
            MarketplaceImportOperation::Install,
            vec![document("not a workflow document")],
        );

        assert!(
            matches!(
                outcomes.as_slice(),
                [MarketplaceWorkflowOutcome::Failed {
                    plugin_id,
                    source_file,
                    reason
                }] if plugin_id == "official/ora.cdase-workflow"
                    && source_file == "assets/workflows/1.0.0.json"
                    && reason.starts_with("document is not valid JSON")
            ),
            "the refusal is reported while the record stays on the fallback path: {outcomes:?}"
        );
        assert_eq!(*sources.forget_calls.lock().expect("forget_calls lock"), 1);
    }
}

//! Persists the outcomes of the workflow documents a marketplace package delivered.
//!
//! The record is owned by the marketplace import, not by the workflow aggregate: it has to outlive
//! the copy it describes so upgrading, reinstalling, or deleting the workflow cannot make the same
//! source path look new again. For that reason nothing here cascades a delete from `workflows`.
//!
//! Every write states the source state the caller read, and a write whose expectation no longer
//! holds is refused instead of applied. That is what keeps a losing writer from overwriting the
//! winner and from reporting a name the library already uses as an import failure.

use ora_application::{
    ImportCommit, NewImportedWorkflow, RepositoryError, SourcePrecondition, WorkflowDocumentSource,
    WorkflowDocumentSourceRecord, WorkflowDocumentSourceRepository,
};
use rusqlite::{Transaction, TransactionBehavior, params};

use crate::DatabaseError;
use crate::repository::RepositoryPool;
use crate::repository::connection::bool_to_sqlite;

/// Stores one state per marketplace workflow document source path.
#[derive(Clone, Debug)]
pub struct SqliteWorkflowDocumentSourceRepository {
    pool: RepositoryPool,
}

impl SqliteWorkflowDocumentSourceRepository {
    /// Builds the source record repository from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl WorkflowDocumentSourceRepository for SqliteWorkflowDocumentSourceRepository {
    fn find_record(
        &self,
        source: &WorkflowDocumentSource,
    ) -> Result<Option<WorkflowDocumentSourceRecord>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                // Liveness is resolved on every read rather than stored, so deleting or restoring
                // the copy beside the record needs no write here.
                let mut statement = connection.prepare(
                    "SELECT wdi.state, (w.id IS NOT NULL AND w.is_deleted = 0) AS copy_alive
                     FROM workflow_document_imports wdi
                     LEFT JOIN workflows w ON w.id = wdi.workflow_id
                     WHERE wdi.source_namespace = ?1 AND wdi.identifier = ?2 AND wdi.document_path = ?3",
                )?;
                let mut rows = statement.query(params![
                    source.namespace,
                    source.identifier,
                    source.document_path
                ])?;
                let recorded = rows
                    .next()?
                    .map(|row| {
                        Ok::<_, DatabaseError>((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                    })
                    .transpose()?;
                recorded
                    .map(|(state, copy_alive)| record_from_state(&state, copy_alive))
                    .transpose()
            })
            .map_err(RepositoryError::new)
    }

    fn record_conflict_skip(
        &self,
        source: &WorkflowDocumentSource,
        recorded_at: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| {
                // The upsert keeps a conflict record current without ever taking an `imported` row
                // away from a copy the user still has: a path that delivered a surviving copy stays
                // delivered. A record whose copy was deleted has nothing left to protect, so it
                // becomes the conflict this cycle can retry from.
                connection.execute(
                    "INSERT INTO workflow_document_imports
                         (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                     VALUES (?1, ?2, ?3, 'conflict_skipped', NULL, ?4)
                     ON CONFLICT (source_namespace, identifier, document_path) DO UPDATE
                         SET state = 'conflict_skipped', workflow_id = NULL, recorded_at = excluded.recorded_at
                         WHERE workflow_document_imports.state = 'conflict_skipped'
                             OR NOT EXISTS(
                                 SELECT 1 FROM workflows w
                                 WHERE w.id = workflow_document_imports.workflow_id AND w.is_deleted = 0
                             )",
                    params![
                        source.namespace,
                        source.identifier,
                        source.document_path,
                        recorded_at
                    ],
                )?;
                Ok(())
            })
            .map_err(RepositoryError::new)
    }

    fn commit_import(
        &self,
        imported: NewImportedWorkflow,
        precondition: SourcePrecondition,
    ) -> Result<ImportCommit, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                // One transaction covers the source transition, the copy, its two snapshots, and
                // the published pointer. A failure therefore rolls all of them back, so a retry can
                // never find a copy without its record, or a record without its copy.
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                // The name is checked inside the transaction with the same case-insensitive rule
                // as the active-name index, so a name taken after the caller looked is reported as
                // the conflict it is instead of surfacing as an import failure.
                let name_taken: bool = transaction.query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM workflows
                         WHERE namespace = ?1 COLLATE NOCASE AND name = ?2 COLLATE NOCASE AND is_deleted = 0
                     )",
                    params![
                        imported.workflow.namespace.as_ref(),
                        &imported.workflow.name
                    ],
                    |row| row.get(0),
                )?;
                if name_taken {
                    return Ok(ImportCommit::NameTaken);
                }
                let transitioned = match precondition {
                    SourcePrecondition::Unrecorded => transaction.execute(
                        "INSERT INTO workflow_document_imports
                             (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                         VALUES (?1, ?2, ?3, 'imported', ?4, ?5)
                         ON CONFLICT (source_namespace, identifier, document_path) DO NOTHING",
                        params![
                            imported.source.namespace,
                            imported.source.identifier,
                            imported.source.document_path,
                            imported.workflow.id.as_ref(),
                            imported.workflow.audit_fields.created_at,
                        ],
                    )? == 1,
                    SourcePrecondition::ConflictSkipped => transaction.execute(
                        "UPDATE workflow_document_imports
                             SET state = 'imported', workflow_id = ?4, recorded_at = ?5
                         WHERE source_namespace = ?1 AND identifier = ?2 AND document_path = ?3
                             AND state = 'conflict_skipped'",
                        params![
                            imported.source.namespace,
                            imported.source.identifier,
                            imported.source.document_path,
                            imported.workflow.id.as_ref(),
                            imported.workflow.audit_fields.created_at,
                        ],
                    )? == 1,
                    SourcePrecondition::DeletedCopy => transaction.execute(
                        "UPDATE workflow_document_imports
                             SET state = 'imported', workflow_id = ?4, recorded_at = ?5
                         WHERE source_namespace = ?1 AND identifier = ?2 AND document_path = ?3
                             AND state = 'imported'
                             AND NOT EXISTS(
                                 SELECT 1 FROM workflows w
                                 WHERE w.id = workflow_document_imports.workflow_id AND w.is_deleted = 0
                             )",
                        params![
                            imported.source.namespace,
                            imported.source.identifier,
                            imported.source.document_path,
                            imported.workflow.id.as_ref(),
                            imported.workflow.audit_fields.created_at,
                        ],
                    )? == 1,
                };
                if !transitioned {
                    return Ok(ImportCommit::SourceChanged);
                }
                transaction.execute(
                    "INSERT INTO workflows (id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6)",
                    params![
                        imported.workflow.id.as_ref(),
                        imported.workflow.namespace.as_ref(),
                        &imported.workflow.name,
                        imported.workflow.audit_fields.created_at,
                        imported.workflow.audit_fields.updated_at,
                        bool_to_sqlite(imported.workflow.audit_fields.is_deleted),
                    ],
                )?;
                for snapshot in [&imported.draft, &imported.published] {
                    transaction.execute(
                        "INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at, updated_at, is_deleted)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            snapshot.id.as_ref(),
                            snapshot.workflow_id.as_ref(),
                            &snapshot.version,
                            &snapshot.graph,
                            snapshot.created_at,
                            snapshot.updated_at,
                            bool_to_sqlite(snapshot.is_deleted),
                        ],
                    )?;
                }
                transaction.execute(
                    "UPDATE workflows SET published_snapshot_id = ?2 WHERE id = ?1",
                    params![
                        imported.workflow.id.as_ref(),
                        imported.published.id.as_ref()
                    ],
                )?;
                transaction.commit()?;
                Ok(ImportCommit::Committed)
            })
            .map_err(RepositoryError::new)
    }

    fn forget_deleted_copy(
        &self,
        source: &WorkflowDocumentSource,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                // Only a record whose copy is really gone is cleared; a record another operation
                // already refreshed stays, and the caller re-judges the document against it.
                let cleared = connection.execute(
                    "DELETE FROM workflow_document_imports
                     WHERE source_namespace = ?1 AND identifier = ?2 AND document_path = ?3
                         AND state = 'imported'
                         AND NOT EXISTS(
                             SELECT 1 FROM workflows w
                             WHERE w.id = workflow_document_imports.workflow_id AND w.is_deleted = 0
                         )",
                    params![source.namespace, source.identifier, source.document_path],
                )?;
                Ok(cleared == 1)
            })
            .map_err(RepositoryError::new)
    }
}

/// Turns the stored state and the resolved copy liveness into the record the caller acts on.
///
/// The column is constrained to the two stored spellings, so an unrecognized value means the row
/// was written by something other than this adapter; reporting it as corrupt keeps that visible
/// instead of silently treating a damaged record as absent and re-importing the path.
fn record_from_state(
    state: &str,
    copy_alive: bool,
) -> Result<WorkflowDocumentSourceRecord, DatabaseError> {
    match (state, copy_alive) {
        ("imported", true) => Ok(WorkflowDocumentSourceRecord::ImportedAlive),
        ("imported", false) => Ok(WorkflowDocumentSourceRecord::ImportedDeleted),
        ("conflict_skipped", _) => Ok(WorkflowDocumentSourceRecord::ConflictSkipped),
        (other, _) => Err(DatabaseError::CorruptWorkflowDocumentSourceState(
            other.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use ora_application::{
        ImportCommit, NewImportedWorkflow, SourcePrecondition, WorkflowDocumentSource,
        WorkflowDocumentSourceRecord, WorkflowDocumentSourceRepository,
    };
    use ora_domain::{
        AuditFields, Namespace, Workflow, WorkflowId, WorkflowSnapshot, WorkflowSnapshotId,
    };
    use pretty_assertions::assert_eq;

    use crate::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog,
    };

    use super::SqliteWorkflowDocumentSourceRepository;

    /// Opens an in-memory pool with the shipped schema so the record table exists.
    fn test_pool() -> RepositoryPool {
        DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap database")
    }

    /// Returns the source path used by the record tests.
    fn source(path: &str) -> WorkflowDocumentSource {
        WorkflowDocumentSource {
            namespace: "official".to_string(),
            identifier: "ora.cdase-workflow".to_string(),
            document_path: path.to_string(),
        }
    }

    /// Builds one importable document with a name the tests can distinguish.
    fn imported_workflow(document_path: &str, name: &str) -> NewImportedWorkflow {
        let now = 1_776_244_428_000;
        let workflow_id = WorkflowId::new(format!("workflow-{name}"));
        let workflow = Workflow::new(
            workflow_id.clone(),
            Namespace::local(),
            name,
            /*published_snapshot_id*/ None,
            AuditFields::new(now, now, /*is_deleted*/ false),
        )
        .expect("valid workflow");
        let draft = WorkflowSnapshot::new(
            WorkflowSnapshotId::new(format!("draft-{name}")),
            workflow_id.clone(),
            "draft",
            "{\"nodes\":[],\"edges\":[]}",
            now,
            Some(now),
            /*is_deleted*/ false,
        );
        let published = WorkflowSnapshot::new(
            WorkflowSnapshotId::new(format!("published-{name}")),
            workflow_id,
            "1.0.0",
            "{\"nodes\":[],\"edges\":[]}",
            now,
            Some(now),
            /*is_deleted*/ false,
        );
        NewImportedWorkflow {
            source: source(document_path),
            workflow,
            draft,
            published,
        }
    }

    /// Soft-deletes every workflow in the pool, as deleting the imported copy does.
    fn delete_copies(pool: &RepositoryPool) {
        pool.with_connection(|connection| {
            connection.execute("UPDATE workflows SET is_deleted = 1", [])?;
            connection.execute("UPDATE workflow_snapshots SET is_deleted = 1", [])?;
            Ok(())
        })
        .expect("delete copies");
    }

    #[test]
    fn records_a_conflict_skip_for_an_unrecorded_path() {
        let repository = SqliteWorkflowDocumentSourceRepository::new(test_pool());
        let source = source("assets/workflows/1.0.0.json");
        assert_eq!(
            repository.find_record(&source).expect("lookup"),
            None,
            "a path starts unrecorded"
        );

        repository.record_conflict_skip(&source, 7).expect("record");

        assert_eq!(
            repository.find_record(&source).expect("lookup"),
            Some(WorkflowDocumentSourceRecord::ConflictSkipped)
        );
    }

    #[test]
    fn keeps_a_conflict_record_current_without_erasing_an_import() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool);
        let source = source("assets/workflows/1.0.0.json");
        repository.record_conflict_skip(&source, 7).expect("record");
        repository
            .record_conflict_skip(&source, 9)
            .expect("re-record");

        assert_eq!(
            repository.find_record(&source).expect("lookup"),
            Some(WorkflowDocumentSourceRecord::ConflictSkipped),
            "a repeated conflict stays one record"
        );

        repository
            .commit_import(
                imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                SourcePrecondition::ConflictSkipped,
            )
            .expect("commit import");
        repository
            .record_conflict_skip(&source, 11)
            .expect("record a late conflict");

        assert_eq!(
            repository.find_record(&source).expect("lookup"),
            Some(WorkflowDocumentSourceRecord::ImportedAlive),
            "a late conflict must not take the delivered copy's record away"
        );
    }

    #[test]
    fn records_a_conflict_for_a_copy_the_user_deleted() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool.clone());
        let source = source("assets/workflows/1.0.0.json");
        repository
            .commit_import(
                imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                SourcePrecondition::Unrecorded,
            )
            .expect("commit import");
        delete_copies(&pool);

        repository
            .record_conflict_skip(&source, 11)
            .expect("record");

        assert_eq!(
            repository.find_record(&source).expect("lookup"),
            Some(WorkflowDocumentSourceRecord::ConflictSkipped),
            "a deleted copy has nothing left to protect, so the conflict replaces its record"
        );
    }

    #[test]
    fn commits_the_copy_and_its_record_together() {
        let repository = SqliteWorkflowDocumentSourceRepository::new(test_pool());
        assert_eq!(
            repository
                .commit_import(
                    imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                    SourcePrecondition::Unrecorded,
                )
                .expect("commit import"),
            ImportCommit::Committed
        );

        assert_eq!(
            repository
                .find_record(&source("assets/workflows/1.0.0.json"))
                .expect("lookup"),
            Some(WorkflowDocumentSourceRecord::ImportedAlive)
        );
    }

    #[test]
    fn refuses_a_transition_whose_precondition_no_longer_holds() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool.clone());
        repository
            .commit_import(
                imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                SourcePrecondition::Unrecorded,
            )
            .expect("commit import");
        let mut second = imported_workflow("assets/workflows/1.0.0.json", "另一份");
        second.workflow.id = WorkflowId::new("workflow-second");

        assert_eq!(
            repository
                .commit_import(second, SourcePrecondition::Unrecorded)
                .expect("second commit"),
            ImportCommit::SourceChanged,
            "another write already claimed the path"
        );
        let copies: i64 = pool
            .with_connection(|connection| {
                Ok(connection.query_row("SELECT COUNT(*) FROM workflows", [], |row| row.get(0))?)
            })
            .expect("count workflows");
        assert_eq!(copies, 1, "the refused commit leaves no second copy");
    }

    #[test]
    fn reports_a_name_the_library_already_uses_instead_of_a_failure() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool.clone());
        repository
            .commit_import(
                imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                SourcePrecondition::Unrecorded,
            )
            .expect("commit import");
        let mut clash = imported_workflow("assets/workflows/2.0.0.json", "发布流程");
        clash.workflow.id = WorkflowId::new("workflow-clash");
        clash.draft.workflow_id = clash.workflow.id.clone();
        clash.published.workflow_id = clash.workflow.id.clone();

        assert_eq!(
            repository
                .commit_import(clash, SourcePrecondition::Unrecorded)
                .expect("commit the clashing document"),
            ImportCommit::NameTaken,
            "the name is compared case-insensitively, as the active-name index does"
        );
        assert_eq!(
            repository
                .find_record(&source("assets/workflows/2.0.0.json"))
                .expect("lookup"),
            None,
            "a refused name leaves the path retryable"
        );
    }

    #[test]
    fn a_failed_commit_leaves_neither_a_copy_nor_a_record() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool.clone());
        let mut imported = imported_workflow("assets/workflows/1.0.0.json", "发布流程");
        // Two snapshots of one workflow cannot both be the draft, so the second insert violates
        // the version index and the whole transaction has to roll back.
        imported.published.version = "draft".to_string();

        assert!(
            repository
                .commit_import(imported, SourcePrecondition::Unrecorded)
                .is_err()
        );

        assert_eq!(
            repository
                .find_record(&source("assets/workflows/1.0.0.json"))
                .expect("lookup"),
            None,
            "a rolled-back import records nothing"
        );
        let copies: i64 = pool
            .with_connection(|connection| {
                Ok(connection.query_row("SELECT COUNT(*) FROM workflows", [], |row| row.get(0))?)
            })
            .expect("count workflows");
        assert_eq!(copies, 0, "a rolled-back import leaves no copy behind");
    }

    #[test]
    fn a_record_survives_the_workflow_delete_and_a_later_reinstall() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool.clone());
        repository
            .commit_import(
                imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                SourcePrecondition::Unrecorded,
            )
            .expect("commit import");
        // Deleting the workflow is a soft delete of the copy; the source record deliberately has
        // no foreign key, so the path stays recorded and only its liveness changes.
        delete_copies(&pool);

        assert_eq!(
            repository
                .find_record(&source("assets/workflows/1.0.0.json"))
                .expect("lookup"),
            Some(WorkflowDocumentSourceRecord::ImportedDeleted),
            "deleting the copy must turn the record into one a reinstall can recover"
        );
    }

    #[test]
    fn forgets_a_deleted_copies_record_only_while_the_copy_stays_deleted() {
        let pool = test_pool();
        let repository = SqliteWorkflowDocumentSourceRepository::new(pool.clone());
        repository
            .commit_import(
                imported_workflow("assets/workflows/1.0.0.json", "发布流程"),
                SourcePrecondition::Unrecorded,
            )
            .expect("commit import");
        assert_eq!(
            repository
                .forget_deleted_copy(&source("assets/workflows/1.0.0.json"))
                .expect("forget against a surviving copy"),
            false,
            "a surviving copy keeps its record"
        );

        delete_copies(&pool);
        assert_eq!(
            repository
                .forget_deleted_copy(&source("assets/workflows/1.0.0.json"))
                .expect("forget against a deleted copy"),
            true
        );
        assert_eq!(
            repository
                .find_record(&source("assets/workflows/1.0.0.json"))
                .expect("lookup"),
            None,
            "a forgotten path is importable again"
        );
    }
}

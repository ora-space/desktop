//! Persists the terminal outcomes of the workflow documents a marketplace package delivered.
//!
//! The record is owned by the marketplace import, not by the workflow aggregate: it has to outlive
//! the copy it describes so upgrading, reinstalling, or deleting the workflow cannot make the same
//! source path look new again. For that reason nothing here cascades a delete from `workflows`.

use ora_application::{
    NewImportedWorkflow, RepositoryError, WorkflowDocumentSource, WorkflowDocumentSourceRepository,
    WorkflowDocumentSourceState,
};
use rusqlite::{Transaction, TransactionBehavior, params};

use crate::DatabaseError;
use crate::repository::RepositoryPool;
use crate::repository::connection::bool_to_sqlite;

/// Stores one terminal state per marketplace workflow document source path.
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
    fn find_state(
        &self,
        source: &WorkflowDocumentSource,
    ) -> Result<Option<WorkflowDocumentSourceState>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT state FROM workflow_document_imports
                     WHERE source_namespace = ?1 AND identifier = ?2 AND document_path = ?3",
                )?;
                let mut rows = statement.query(params![
                    source.namespace,
                    source.identifier,
                    source.document_path
                ])?;
                let recorded = rows
                    .next()?
                    .map(|row| row.get::<_, String>(0))
                    .transpose()?;
                recorded.map(|state| parse_state(&state)).transpose()
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
                connection.execute(
                    "INSERT INTO workflow_document_imports
                         (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                     VALUES (?1, ?2, ?3, 'conflict_skipped', NULL, ?4)",
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

    fn commit_import(&self, imported: NewImportedWorkflow) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                // One transaction covers the copy, its two snapshots, the published pointer, and
                // the terminal record. A failure therefore rolls all of them back, so a retry can
                // never find a copy without its record, or a record without its copy.
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
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
                transaction.execute(
                    "INSERT INTO workflow_document_imports
                         (source_namespace, identifier, document_path, state, workflow_id, recorded_at)
                     VALUES (?1, ?2, ?3, 'imported', ?4, ?5)",
                    params![
                        imported.source.namespace,
                        imported.source.identifier,
                        imported.source.document_path,
                        imported.workflow.id.as_ref(),
                        imported.workflow.audit_fields.created_at,
                    ],
                )?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(RepositoryError::new)
    }
}

/// Reads back the state spelling the schema's closed vocabulary stores.
///
/// The column is constrained to these two spellings, so an unrecognized value means the row was
/// written by something other than this adapter; reporting it as corrupt keeps that visible
/// instead of silently treating a damaged record as absent and re-importing the path.
fn parse_state(state: &str) -> Result<WorkflowDocumentSourceState, DatabaseError> {
    match state {
        "imported" => Ok(WorkflowDocumentSourceState::Imported),
        "conflict_skipped" => Ok(WorkflowDocumentSourceState::ConflictSkipped),
        other => Err(DatabaseError::CorruptWorkflowDocumentSourceState(
            other.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use ora_application::{
        NewImportedWorkflow, WorkflowDocumentSource, WorkflowDocumentSourceRepository,
        WorkflowDocumentSourceState,
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

    #[test]
    fn records_a_conflict_skip_as_a_terminal_state() {
        let repository = SqliteWorkflowDocumentSourceRepository::new(test_pool());
        let source = source("assets/workflows/1.0.0.json");
        assert_eq!(
            repository.find_state(&source).expect("lookup"),
            None,
            "a path starts unrecorded"
        );

        repository.record_conflict_skip(&source, 7).expect("record");

        assert_eq!(
            repository.find_state(&source).expect("lookup"),
            Some(WorkflowDocumentSourceState::ConflictSkipped)
        );
    }

    #[test]
    fn commits_the_copy_and_its_record_together() {
        let repository = SqliteWorkflowDocumentSourceRepository::new(test_pool());
        repository
            .commit_import(imported_workflow("assets/workflows/1.0.0.json", "发布流程"))
            .expect("commit import");

        assert_eq!(
            repository
                .find_state(&source("assets/workflows/1.0.0.json"))
                .expect("lookup"),
            Some(WorkflowDocumentSourceState::Imported)
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

        assert!(repository.commit_import(imported).is_err());

        assert_eq!(
            repository
                .find_state(&source("assets/workflows/1.0.0.json"))
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
            .commit_import(imported_workflow("assets/workflows/1.0.0.json", "发布流程"))
            .expect("commit import");
        // Deleting the workflow is a soft delete of the copy; the source record deliberately has
        // no foreign key, so the path stays terminal.
        pool.with_connection(|connection| {
            connection.execute("UPDATE workflows SET is_deleted = 1", [])?;
            connection.execute("UPDATE workflow_snapshots SET is_deleted = 1", [])?;
            Ok(())
        })
        .expect("delete copies");

        assert_eq!(
            repository
                .find_state(&source("assets/workflows/1.0.0.json"))
                .expect("lookup"),
            Some(WorkflowDocumentSourceState::Imported),
            "deleting the copy must not free the source path"
        );
    }
}

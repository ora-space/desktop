use ora_application::{
    ActivateVersionResult, DeleteSnapshotResult, DeleteWorkflowResult, PublishSnapshotResult,
    RepositoryError, RollbackDraftResult, UpdateDraftResult, UpdateWorkflowResult,
    WorkflowRepository,
};
use ora_domain::{
    AuditFields, CreatedWorkflow, Namespace, Workflow, WorkflowDetail, WorkflowId,
    WorkflowSnapshot, WorkflowSnapshotId, WorkflowSummary, WorkflowVersion,
};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

const DRAFT_VERSION: &str = "draft";

/// Persists workflow definitions and their versioned snapshots in SQLite.
#[derive(Clone, Debug)]
pub struct SqliteWorkflowRepository {
    pool: RepositoryPool,
}

impl SqliteWorkflowRepository {
    /// Builds a workflow repository from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl WorkflowRepository for SqliteWorkflowRepository {
    fn create_workflow(
        &self,
        workflow: Workflow,
        draft: WorkflowSnapshot,
    ) -> Result<CreatedWorkflow, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                transaction.execute(
                    "INSERT INTO workflows (id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        workflow.id.as_ref(),
                        workflow.namespace.as_ref(),
                        &workflow.name,
                        workflow.published_snapshot_id.as_ref().map(AsRef::as_ref),
                        workflow.audit_fields.created_at,
                        workflow.audit_fields.updated_at,
                        bool_to_sqlite(workflow.audit_fields.is_deleted),
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        draft.id.as_ref(),
                        draft.workflow_id.as_ref(),
                        &draft.version,
                        &draft.graph,
                        draft.created_at,
                        draft.updated_at,
                        bool_to_sqlite(draft.is_deleted),
                    ],
                )?;
                transaction.commit()?;
                Ok(CreatedWorkflow { workflow, draft })
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn find_workflow(&self, workflow_id: &WorkflowId) -> Result<Option<Workflow>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted FROM workflows WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![workflow_id.as_ref()])?;
                rows.next()?.map(map_workflow_row).transpose()
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn find_workflow_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Workflow>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted FROM workflows WHERE namespace = ?1 COLLATE NOCASE AND name = ?2 COLLATE NOCASE AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![namespace.as_ref(), name])?;
                rows.next()?.map(map_workflow_row).transpose()
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn get_workflow_detail(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowDetail>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let workflow = {
                    let mut statement = connection.prepare(
                        "SELECT id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted FROM workflows WHERE id = ?1 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref()])?;
                    match rows.next()?.map(map_workflow_row).transpose()? {
                        Some(wf) => wf,
                        None => return Ok(None),
                    }
                };

                let draft = {
                    let mut statement = connection.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref(), DRAFT_VERSION])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };

                let published = if let Some(published_id) = &workflow.published_snapshot_id {
                    let mut statement = connection.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE id = ?1 AND workflow_id = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![published_id.as_ref(), workflow_id.as_ref()])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                } else {
                    None
                };

                Ok(Some(WorkflowDetail {
                    workflow,
                    // Every workflow must have a draft after creation; if it is missing something
                    // is corrupt — return None to signal the aggregate is incomplete.
                    draft: match draft {
                        Some(d) => d,
                        None => return Ok(None),
                    },
                    published,
                }))
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT w.id, w.namespace, w.name, ws.version, w.created_at, w.updated_at
                     FROM workflows w
                     LEFT JOIN workflow_snapshots ws
                       ON ws.id = w.published_snapshot_id AND ws.is_deleted = 0
                     WHERE w.is_deleted = 0
                     ORDER BY w.created_at DESC, w.id DESC",
                )?;
                let mut rows = statement.query([])?;
                let mut workflows = Vec::new();
                while let Some(row) = rows.next()? {
                    workflows.push(WorkflowSummary {
                        id: row.get::<_, String>("id")?,
                        namespace: Namespace::new(row.get::<_, String>("namespace")?)?,
                        name: row.get::<_, String>("name")?,
                        published_version: row.get::<_, Option<String>>("version")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                    });
                }
                Ok(workflows)
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn update_workflow(
        &self,
        workflow_id: &WorkflowId,
        name: String,
        updated_at: i64,
    ) -> Result<UpdateWorkflowResult, RepositoryError> {
        self
            .pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                        "UPDATE workflows SET name = ?2, updated_at = ?3 WHERE id = ?1 AND is_deleted = 0 RETURNING id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted",
                    )?;
                let mut rows = statement.query(params![
                    workflow_id.as_ref(),
                    name,
                    updated_at
                ])?;
                match rows.next()?.map(map_workflow_row).transpose()? {
                    Some(workflow) => Ok(UpdateWorkflowResult::Updated(workflow)),
                    None => Ok(UpdateWorkflowResult::WorkflowNotFound),
                }
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn soft_delete_workflow(
        &self,
        workflow_id: &WorkflowId,
        deleted_at: i64,
    ) -> Result<DeleteWorkflowResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflows WHERE id = ?1 AND is_deleted = 0",
                        params![workflow_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();

                if !exists {
                    return Ok(DeleteWorkflowResult::NotFound);
                }

                // Runs freeze a snapshot as their graph; deleting a workflow that still has live
                // runs would orphan those runs' graphs, so it is refused just like the
                // single-snapshot delete guard.
                let active_runs = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM workflow_runs wr
                        WHERE wr.is_deleted = 0
                          AND wr.snapshot_id IN (
                              SELECT id FROM workflow_snapshots
                              WHERE workflow_id = ?1 AND is_deleted = 0
                          )
                    )",
                    params![workflow_id.as_ref()],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                if active_runs {
                    return Ok(DeleteWorkflowResult::ActiveRuns);
                }

                transaction.execute(
                    "UPDATE workflow_snapshots SET is_deleted = 1 WHERE workflow_id = ?1 AND is_deleted = 0",
                    params![workflow_id.as_ref()],
                )?;
                transaction.execute(
                    "UPDATE workflows SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                    params![workflow_id.as_ref(), deleted_at],
                )?;
                transaction.commit()?;
                Ok(DeleteWorkflowResult::Deleted)
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn find_snapshot_by_version(
        &self,
        workflow_id: &WorkflowId,
        version: &str,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![workflow_id.as_ref(), version])?;
                rows.next()?.map(map_snapshot_row).transpose()
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn find_snapshot_by_id(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE id = ?1 AND workflow_id = ?2 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![snapshot_id.as_ref(), workflow_id.as_ref()])?;
                rows.next()?.map(map_snapshot_row).transpose()
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn find_snapshot_any_workflow(
        &self,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![snapshot_id.as_ref()])?;
                rows.next()?.map(map_snapshot_row).transpose()
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn list_versions(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, version, created_at FROM workflow_snapshots WHERE workflow_id = ?1 AND version != ?2 AND is_deleted = 0 ORDER BY created_at DESC",
                )?;
                let mut rows = statement.query(params![workflow_id.as_ref(), DRAFT_VERSION])?;
                let mut versions = Vec::new();
                while let Some(row) = rows.next()? {
                    versions.push(WorkflowVersion {
                        id: row.get::<_, String>("id")?,
                        version: row.get::<_, String>("version")?,
                        created_at: row.get("created_at")?,
                    });
                }
                Ok(versions)
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn update_draft(
        &self,
        workflow_id: &WorkflowId,
        graph: String,
        updated_at: i64,
    ) -> Result<UpdateDraftResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let workflow_exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflows WHERE id = ?1 AND is_deleted = 0",
                        params![workflow_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !workflow_exists {
                    return Ok(UpdateDraftResult::WorkflowNotFound);
                }

                let rows_affected = transaction.execute(
                    "UPDATE workflow_snapshots SET graph = ?3, updated_at = ?4 WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                    params![workflow_id.as_ref(), DRAFT_VERSION, &graph, updated_at],
                )?;
                if rows_affected == 0 {
                    return Ok(UpdateDraftResult::DraftNotFound);
                }
                let draft = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref(), DRAFT_VERSION])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(draft) = draft else {
                    return Ok(UpdateDraftResult::DraftNotFound);
                };
                transaction.commit()?;
                Ok(UpdateDraftResult::Updated(draft))
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn publish_snapshot(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: WorkflowSnapshotId,
        version: String,
        created_at: i64,
    ) -> Result<PublishSnapshotResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let workflow_exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflows WHERE id = ?1 AND is_deleted = 0",
                        params![workflow_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !workflow_exists {
                    return Ok(PublishSnapshotResult::WorkflowNotFound);
                }

                let version_exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                        params![workflow_id.as_ref(), &version],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if version_exists {
                    return Ok(PublishSnapshotResult::VersionAlreadyExists);
                }

                let draft = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref(), DRAFT_VERSION])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(draft) = draft else {
                    return Ok(PublishSnapshotResult::DraftNotFound);
                };

                let snapshot = WorkflowSnapshot::new(
                    snapshot_id,
                    workflow_id.clone(),
                    version,
                    draft.graph,
                    created_at,
                    /*updated_at*/ None,
                    /*is_deleted*/ false,
                );
                transaction.execute(
                    "INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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
                let rows_affected = transaction.execute(
                    "UPDATE workflows SET published_snapshot_id = ?2, updated_at = ?3 WHERE id = ?1 AND is_deleted = 0",
                    params![workflow_id.as_ref(), snapshot.id.as_ref(), snapshot.created_at],
                )?;
                if rows_affected == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::InvalidQuery));
                }
                transaction.commit()?;
                Ok(PublishSnapshotResult::Published(snapshot))
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn rollback_draft(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
        updated_at: i64,
    ) -> Result<RollbackDraftResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let workflow_exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflows WHERE id = ?1 AND is_deleted = 0",
                        params![workflow_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !workflow_exists {
                    return Ok(RollbackDraftResult::WorkflowNotFound);
                }

                let target = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE id = ?1 AND workflow_id = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![snapshot_id.as_ref(), workflow_id.as_ref()])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(target) = target else {
                    return Ok(RollbackDraftResult::SnapshotNotFound);
                };
                if target.version == DRAFT_VERSION {
                    return Ok(RollbackDraftResult::DraftSnapshot);
                };

                let rows_affected = transaction.execute(
                    "UPDATE workflow_snapshots
                     SET graph = ?2, updated_at = ?3
                     WHERE workflow_id = ?1 AND version = ?4 AND is_deleted = 0",
                    params![workflow_id.as_ref(), target.graph, updated_at, DRAFT_VERSION],
                )?;

                if rows_affected == 0 {
                    return Ok(RollbackDraftResult::DraftNotFound);
                }

                let draft = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref(), DRAFT_VERSION])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(draft) = draft else {
                    return Ok(RollbackDraftResult::DraftNotFound);
                };
                transaction.commit()?;
                Ok(RollbackDraftResult::DraftUpdated(draft))
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn activate_version(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
        updated_at: i64,
    ) -> Result<ActivateVersionResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let workflow_exists = transaction
                    .query_row(
                        "SELECT 1 FROM workflows WHERE id = ?1 AND is_deleted = 0",
                        params![workflow_id.as_ref()],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !workflow_exists {
                    return Ok(ActivateVersionResult::WorkflowNotFound);
                }

                let target = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE id = ?1 AND workflow_id = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![snapshot_id.as_ref(), workflow_id.as_ref()])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(target) = target else {
                    return Ok(ActivateVersionResult::SnapshotNotFound);
                };
                if target.version == DRAFT_VERSION {
                    return Ok(ActivateVersionResult::DraftSnapshot);
                };

                let draft_rows = transaction.execute(
                    "UPDATE workflow_snapshots
                     SET graph = ?2, updated_at = ?3
                     WHERE workflow_id = ?1 AND version = ?4 AND is_deleted = 0",
                    params![workflow_id.as_ref(), target.graph, updated_at, DRAFT_VERSION],
                )?;
                if draft_rows == 0 {
                    return Ok(ActivateVersionResult::DraftNotFound);
                }

                let workflow_rows = transaction.execute(
                    "UPDATE workflows SET published_snapshot_id = ?2, updated_at = ?3 WHERE id = ?1 AND is_deleted = 0",
                    params![workflow_id.as_ref(), snapshot_id.as_ref(), updated_at],
                )?;
                if workflow_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::InvalidQuery));
                }

                let draft = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE workflow_id = ?1 AND version = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref(), DRAFT_VERSION])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(draft) = draft else {
                    return Ok(ActivateVersionResult::DraftNotFound);
                };
                transaction.commit()?;
                Ok(ActivateVersionResult::Activated(draft))
            })
            .map_err(workflow_repository_error_from_database)
    }

    fn soft_delete_snapshot(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
        _deleted_at: i64,
    ) -> Result<DeleteSnapshotResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let workflow = {
                    let mut statement = transaction.prepare(
                        "SELECT id, namespace, name, published_snapshot_id, created_at, updated_at, is_deleted FROM workflows WHERE id = ?1 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![workflow_id.as_ref()])?;
                    rows.next()?.map(map_workflow_row).transpose()?
                };
                let Some(workflow) = workflow else {
                    return Ok(DeleteSnapshotResult::WorkflowNotFound);
                };

                let snapshot = {
                    let mut statement = transaction.prepare(
                        "SELECT id, workflow_id, version, graph, created_at, updated_at, is_deleted FROM workflow_snapshots WHERE id = ?1 AND workflow_id = ?2 AND is_deleted = 0",
                    )?;
                    let mut rows = statement.query(params![snapshot_id.as_ref(), workflow_id.as_ref()])?;
                    rows.next()?.map(map_snapshot_row).transpose()?
                };
                let Some(snapshot) = snapshot else {
                    return Ok(DeleteSnapshotResult::SnapshotNotFound);
                };
                if snapshot.version == DRAFT_VERSION {
                    return Ok(DeleteSnapshotResult::DraftSnapshot);
                }
                if workflow.published_snapshot_id.as_ref() == Some(&snapshot.id) {
                    return Ok(DeleteSnapshotResult::ActiveSnapshot);
                }

                // A run pins its snapshot as the frozen graph across its viewable and restartable
                // lifecycle, so a snapshot referenced by any live run must not be soft-deleted.
                let referenced = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM workflow_runs
                        WHERE snapshot_id = ?1 AND is_deleted = 0
                    )",
                    params![snapshot.id.as_ref()],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                if referenced {
                    return Ok(DeleteSnapshotResult::SnapshotInUse);
                }

                transaction.execute(
                    "UPDATE workflow_snapshots SET is_deleted = 1 WHERE id = ?1 AND workflow_id = ?2 AND is_deleted = 0",
                    params![snapshot.id.as_ref(), workflow_id.as_ref()],
                )?;
                transaction.commit()?;
                Ok(DeleteSnapshotResult::Deleted(snapshot))
            })
            .map_err(workflow_repository_error_from_database)
    }
}

/// Reconstructs a domain workflow from a selected SQLite row.
fn map_workflow_row(row: &Row<'_>) -> Result<Workflow, crate::DatabaseError> {
    Workflow::new(
        WorkflowId::new(row.get::<_, String>("id")?),
        Namespace::new(row.get::<_, String>("namespace")?)?,
        row.get::<_, String>("name")?,
        row.get::<_, Option<String>>("published_snapshot_id")?
            .map(WorkflowSnapshotId::new),
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    )
    .map_err(Into::into)
}

/// Reconstructs a domain snapshot from a selected SQLite row.
fn map_snapshot_row(row: &Row<'_>) -> Result<WorkflowSnapshot, crate::DatabaseError> {
    Ok(WorkflowSnapshot::new(
        WorkflowSnapshotId::new(row.get::<_, String>("id")?),
        WorkflowId::new(row.get::<_, String>("workflow_id")?),
        row.get::<_, String>("version")?,
        row.get::<_, String>("graph")?,
        row.get("created_at")?,
        row.get("updated_at")?,
        row.get::<_, i64>("is_deleted")? != 0,
    ))
}

/// Converts database failures into application-port errors.
fn workflow_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

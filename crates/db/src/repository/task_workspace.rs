use ora_application::{RepositoryError, TaskWorkspaceCommit, WorkspaceCommitOutcome};
use ora_domain::{Task, Worktree, WorktreeProvisioningLeaseId};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::repository::RepositoryPool;
use crate::repository::connection::bool_to_sqlite;

/// Commits task creation atomically against concurrent project deletion.
///
/// The task and worktree rows, the project visibility check, and the
/// provisioning lease removal all share one immediate transaction, so a
/// project cascade can never interleave between them: either the cascade sees
/// the committed rows (and registers cleanup jobs for them), or this commit
/// fails with `ProjectNotVisible` and the caller compensates the provisioned
/// Git resources itself.
#[derive(Clone, Debug)]
pub struct SqliteTaskWorkspaceRepository {
    pool: RepositoryPool,
}

impl SqliteTaskWorkspaceRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl TaskWorkspaceCommit for SqliteTaskWorkspaceRepository {
    /// Atomically persists a worktree-backed task and releases its provisioning lease.
    fn commit_worktree_task(
        &self,
        task: &Task,
        worktree: &Worktree,
        lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                if !project_visible(&transaction, task.project_id.as_ref())? {
                    return Ok(WorkspaceCommitOutcome::ProjectNotVisible);
                }
                insert_worktree(&transaction, worktree)?;
                insert_task(&transaction, task)?;
                transaction.execute(
                    "DELETE FROM worktree_provisioning_leases WHERE id = ?1",
                    params![lease_id.as_ref()],
                )?;
                transaction.commit()?;
                Ok(WorkspaceCommitOutcome::Committed)
            })
            .map_err(RepositoryError::new)
    }

    /// Atomically persists a project-root task after re-validating its project.
    fn commit_project_root_task(
        &self,
        task: &Task,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                if !project_visible(&transaction, task.project_id.as_ref())? {
                    return Ok(WorkspaceCommitOutcome::ProjectNotVisible);
                }
                insert_task(&transaction, task)?;
                transaction.commit()?;
                Ok(WorkspaceCommitOutcome::Committed)
            })
            .map_err(RepositoryError::new)
    }
}

/// Reports whether the owning project row is still visible to new descendants.
fn project_visible(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<bool, rusqlite::Error> {
    Ok(transaction
        .query_row(
            "SELECT 1 FROM projects WHERE id = ?1 AND is_deleted = 0",
            params![project_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Inserts one task row inside the open commit transaction.
fn insert_task(transaction: &Transaction<'_>, task: &Task) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO tasks (id, project_id, title, type, workflow_run_id, worktree_id, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            task.id.as_ref(),
            task.project_id.as_ref(),
            &task.title,
            task.task_type.database_value(),
            task.workflow_run_id.as_ref().map(AsRef::as_ref),
            task.worktree_id.as_ref().map(AsRef::as_ref),
            task.audit_fields.created_at,
            task.audit_fields.updated_at,
            bool_to_sqlite(task.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Inserts one worktree row inside the open commit transaction.
fn insert_worktree(
    transaction: &Transaction<'_>,
    worktree: &Worktree,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO worktrees (id, task_id, branch_name, checkout_root, base_commit_id, is_active, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            worktree.id.as_ref(),
            worktree.task_id.as_ref(),
            worktree.branch_name.as_deref(),
            worktree.checkout_root.as_deref(),
            worktree.baseline.commit_id(),
            worktree.activity.database_value(),
            worktree.audit_fields.created_at,
            worktree.audit_fields.updated_at,
            bool_to_sqlite(worktree.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

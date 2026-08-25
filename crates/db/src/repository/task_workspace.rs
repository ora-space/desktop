use ora_application::{RepositoryError, TaskWorkspaceCommit, WorkspaceCommitOutcome};
use ora_domain::{
    Task, Workspace, WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation,
    WorkspaceProvisionerKind, WorkspaceProvisioningState, Worktree, WorktreeProvisioningLeaseId,
};
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
                let checkout_root = transaction.query_row(
                    "SELECT checkout_root FROM worktree_provisioning_leases WHERE id = ?1",
                    params![lease_id.as_ref()],
                    |row| row.get::<_, String>(0),
                )?;
                let workspace = Workspace::new(
                    task.workspace_id.clone(),
                    task.project_id.clone(),
                    WorkspaceKind::Isolated,
                    WorkspaceLocation::local_filesystem(checkout_root),
                    WorkspaceLifecycle::Active,
                    task.audit_fields.clone(),
                );
                insert_workspace(&transaction, &workspace)?;
                insert_provisioning(&transaction, &workspace)?;
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
        "INSERT INTO tasks (id, workspace_id, title, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            task.id.as_ref(),
            task.workspace_id.as_ref(),
            &task.title,
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
        "INSERT INTO worktrees (workspace_id, branch_name, base_commit_id, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            worktree.workspace_id.as_ref(),
            worktree.branch_name.as_deref(),
            worktree.baseline.commit_id(),
            worktree.audit_fields.created_at,
            worktree.audit_fields.updated_at,
            bool_to_sqlite(worktree.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Inserts the workspace identity before its worktree and task projection so all runtime records
/// can reference the workspace directly after this transaction commits.
fn insert_workspace(
    transaction: &Transaction<'_>,
    workspace: &Workspace,
) -> Result<(), rusqlite::Error> {
    let location_id = format!("{}-location", workspace.id);
    let locator = match &workspace.location {
        WorkspaceLocation::LocalFilesystem { path } => {
            serde_json::json!({ "path": path }).to_string()
        }
        WorkspaceLocation::Ssh {
            connection_ref,
            path,
        } => serde_json::json!({ "connection_ref": connection_ref, "path": path }).to_string(),
        WorkspaceLocation::RemoteTarget {
            target_ref,
            locator,
            ..
        } => serde_json::json!({ "target_ref": target_ref, "locator": locator }).to_string(),
    };
    transaction.execute(
        "INSERT INTO workspace_locations (id, location_kind, plugin_id, locator_version, locator_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)",
        params![
            location_id,
            workspace.location.database_kind(),
            match &workspace.location {
                WorkspaceLocation::RemoteTarget { plugin_id, .. } => Some(plugin_id),
                WorkspaceLocation::LocalFilesystem { .. } | WorkspaceLocation::Ssh { .. } => None,
            },
            locator,
            workspace.audit_fields.created_at,
            workspace.audit_fields.updated_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            workspace.id.as_ref(),
            workspace.project_id.as_ref(),
            workspace.kind.database_value(),
            format!("{}-location", workspace.id),
            workspace.lifecycle.database_value(),
            workspace.audit_fields.created_at,
            workspace.audit_fields.updated_at,
            bool_to_sqlite(workspace.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Records a ready local Git provisioning result for an already-created isolated workspace.
fn insert_provisioning(
    transaction: &Transaction<'_>,
    workspace: &Workspace,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO workspace_provisioning (workspace_id, provisioner_kind, state, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            workspace.id.as_ref(),
            WorkspaceProvisionerKind::LocalGit.database_value(),
            WorkspaceProvisioningState::Ready.database_value(),
            workspace.audit_fields.created_at,
            workspace.audit_fields.updated_at,
        ],
    )?;
    Ok(())
}

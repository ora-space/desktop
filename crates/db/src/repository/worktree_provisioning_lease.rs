use ora_application::{RepositoryError, WorktreeProvisioningLeaseStore};
use ora_domain::{
    GitCleanupJob, GitCleanupJobId, ProjectId, WorkspaceId, WorktreeProvisioningLease,
    WorktreeProvisioningLeaseId,
};
use rusqlite::{Row, Transaction, params};
use uuid::Uuid;

use crate::repository::RepositoryPool;
use crate::repository::git_cleanup_job::insert_jobs;

/// Persists write-ahead provisioning leases for in-flight worktree creation.
///
/// A lease exists exactly while `git worktree add` may have produced physical
/// resources that no committed task/worktree row owns yet. The provisioning
/// flow renews the expiry while slow Git work runs; only leases that expired
/// without renewal are reclaimed, which is what distinguishes "slow" from
/// "owner died".
#[derive(Clone, Debug)]
pub struct SqliteWorktreeProvisioningLeaseRepository {
    pool: RepositoryPool,
}

impl SqliteWorktreeProvisioningLeaseRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Persists a new lease before any Git mutation starts.
    pub fn create_lease(
        &self,
        lease: &WorktreeProvisioningLease,
    ) -> Result<(), crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "INSERT INTO worktree_provisioning_leases (
                     id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                     lease_expires_at, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    lease.id.as_ref(),
                    lease.project_id.as_ref(),
                    lease.workspace_id.as_ref(),
                    lease.repository_root,
                    lease.checkout_root,
                    lease.branch_name,
                    lease.lease_expires_at,
                    lease.created_at,
                    lease.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Extends one live lease and reports whether it still existed.
    pub fn renew_lease(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
        lease_expires_at: i64,
        now: i64,
    ) -> Result<bool, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let updated = connection.execute(
                "UPDATE worktree_provisioning_leases
                 SET lease_expires_at = ?2, updated_at = ?3
                 WHERE id = ?1",
                params![lease_id.as_ref(), lease_expires_at, now],
            )?;
            Ok(updated > 0)
        })
    }

    /// Removes one lease after its owner finished (successfully or via compensation).
    pub fn delete_lease(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<bool, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let deleted = connection.execute(
                "DELETE FROM worktree_provisioning_leases WHERE id = ?1",
                params![lease_id.as_ref()],
            )?;
            Ok(deleted > 0)
        })
    }

    /// Lists every lease regardless of expiry, in creation order (tests, diagnostics).
    pub fn list_leases(&self) -> Result<Vec<WorktreeProvisioningLease>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                        lease_expires_at, created_at, updated_at
                 FROM worktree_provisioning_leases
                 ORDER BY created_at, id",
            )?;
            let mut rows = statement.query([])?;
            let mut leases = Vec::new();
            while let Some(row) = rows.next()? {
                leases.push(map_lease_row(row)?);
            }
            Ok(leases)
        })
    }

    /// Converts every expired lease into a pending cleanup job atomically.
    ///
    /// Deleting the lease and inserting its job happen in one transaction so a
    /// concurrent renewal or completion (which deletes the lease first) makes
    /// the reclaim a no-op instead of producing a duplicate cleanup target.
    pub fn reclaim_expired_leases(
        &self,
        now: i64,
    ) -> Result<Vec<GitCleanupJob>, crate::DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                Transaction::new(connection, rusqlite::TransactionBehavior::Immediate)?;
            let expired = {
                let mut statement = transaction.prepare(
                    "SELECT id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                            lease_expires_at, created_at, updated_at
                     FROM worktree_provisioning_leases
                     WHERE lease_expires_at < ?1
                     ORDER BY lease_expires_at, id",
                )?;
                let mut rows = statement.query(params![now])?;
                let mut leases = Vec::new();
                while let Some(row) = rows.next()? {
                    leases.push(map_lease_row(row)?);
                }
                leases
            };

            let mut jobs = Vec::new();
            for lease in expired {
                let lease_id = lease.id.clone();
                let job =
                    lease.into_cleanup_job(GitCleanupJobId::new(Uuid::new_v4().to_string()), now);
                insert_jobs(&transaction, std::slice::from_ref(&job))?;
                transaction.execute(
                    "DELETE FROM worktree_provisioning_leases WHERE id = ?1",
                    params![lease_id.as_ref()],
                )?;
                jobs.push(job);
            }
            transaction.commit()?;
            Ok(jobs)
        })
    }
}

impl WorktreeProvisioningLeaseStore for SqliteWorktreeProvisioningLeaseRepository {
    fn create_lease(&self, lease: &WorktreeProvisioningLease) -> Result<(), RepositoryError> {
        SqliteWorktreeProvisioningLeaseRepository::create_lease(self, lease)
            .map_err(RepositoryError::new)
    }

    fn renew_lease(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
        lease_expires_at: i64,
        now: i64,
    ) -> Result<bool, RepositoryError> {
        SqliteWorktreeProvisioningLeaseRepository::renew_lease(
            self,
            lease_id,
            lease_expires_at,
            now,
        )
        .map_err(RepositoryError::new)
    }

    /// Converts one lease into an immediately pending cleanup job atomically.
    ///
    /// A lease that already disappeared (committed creation or concurrent
    /// reclaim) makes this a no-op: whoever removed it owns the resources now.
    fn release_to_cleanup(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
        now: i64,
    ) -> Result<(), RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(
                    connection,
                    rusqlite::TransactionBehavior::Immediate,
                )?;
                let lease = {
                    let mut statement = transaction.prepare(
                        "SELECT id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                                lease_expires_at, created_at, updated_at
                         FROM worktree_provisioning_leases
                         WHERE id = ?1",
                    )?;
                    let mut rows = statement.query(params![lease_id.as_ref()])?;
                    match rows.next()? {
                        Some(row) => Some(map_lease_row(row)?),
                        None => None,
                    }
                };
                if let Some(lease) = lease {
                    let job = lease
                        .into_cleanup_job(GitCleanupJobId::new(Uuid::new_v4().to_string()), now);
                    insert_jobs(&transaction, std::slice::from_ref(&job))?;
                    transaction.execute(
                        "DELETE FROM worktree_provisioning_leases WHERE id = ?1",
                        params![lease_id.as_ref()],
                    )?;
                }
                transaction.commit()?;
                Ok(())
            })
            .map_err(RepositoryError::new)
    }
}

/// Reconstructs one persisted provisioning lease from its selected columns.
fn map_lease_row(row: &Row<'_>) -> Result<WorktreeProvisioningLease, crate::DatabaseError> {
    Ok(WorktreeProvisioningLease {
        id: WorktreeProvisioningLeaseId::new(row.get::<_, String>("id")?),
        project_id: ProjectId::new(row.get::<_, String>("project_id")?),
        workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        repository_root: row.get("repository_root")?,
        checkout_root: row.get("checkout_root")?,
        branch_name: row.get("branch_name")?,
        lease_expires_at: row.get("lease_expires_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

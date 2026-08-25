use ora_domain::{
    GitCleanupJob, GitCleanupJobId, GitCleanupJobState, ProjectId, WorkspaceId,
    truncate_cleanup_error,
};
use rusqlite::{Row, Transaction, params};

use crate::repository::RepositoryPool;

/// Persists durable Git cleanup jobs and their controlled state transitions.
///
/// Jobs never carry a persisted "running" state: the worker keeps executing
/// jobs in `pending`, so a crash at any point leaves the job replayable by
/// startup reconciliation. All transitions go through the named methods below
/// instead of ad-hoc UPDATEs so the pending → completed / manual_attention
/// state machine stays enforced in one place.
#[derive(Clone, Debug)]
pub struct SqliteGitCleanupJobRepository {
    pool: RepositoryPool,
}

impl SqliteGitCleanupJobRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Inserts one job outside of a cascade transaction (lease reclaim, tests).
    pub fn insert_job(&self, job: &GitCleanupJob) -> Result<(), crate::DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let transaction =
                Transaction::new(connection, rusqlite::TransactionBehavior::Immediate)?;
            insert_jobs(&transaction, std::slice::from_ref(job))?;
            transaction.commit()?;
            Ok(())
        })
    }

    /// Returns the next batch of executable jobs ordered for cross-repository fairness.
    pub fn due_jobs(
        &self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<GitCleanupJob>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                        state, attempts, next_attempt_at, last_attempt_at, last_error,
                        created_at, updated_at
                 FROM git_cleanup_jobs
                 WHERE state = 'pending' AND next_attempt_at <= ?1
                 ORDER BY next_attempt_at, id
                 LIMIT ?2",
            )?;
            let mut rows = statement.query(params![now, limit as i64])?;
            let mut jobs = Vec::new();
            while let Some(row) = rows.next()? {
                jobs.push(map_job_row(row)?);
            }
            Ok(jobs)
        })
    }

    /// Lists every job regardless of state, in creation order (diagnostics and tests).
    pub fn list_jobs(&self) -> Result<Vec<GitCleanupJob>, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                        state, attempts, next_attempt_at, last_attempt_at, last_error,
                        created_at, updated_at
                 FROM git_cleanup_jobs
                 ORDER BY created_at, id",
            )?;
            let mut rows = statement.query([])?;
            let mut jobs = Vec::new();
            while let Some(row) = rows.next()? {
                jobs.push(map_job_row(row)?);
            }
            Ok(jobs)
        })
    }

    /// Marks one pending job as the terminal business success.
    pub fn mark_completed(
        &self,
        job_id: &GitCleanupJobId,
        now: i64,
    ) -> Result<(), crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE git_cleanup_jobs
                 SET state = 'completed', last_attempt_at = ?2, updated_at = ?2
                 WHERE id = ?1 AND state = 'pending'",
                params![job_id.as_ref(), now],
            )?;
            Ok(())
        })
    }

    /// Records one retryable failure and schedules the next attempt.
    pub fn record_retryable_failure(
        &self,
        job_id: &GitCleanupJobId,
        error: &str,
        next_attempt_at: i64,
        now: i64,
    ) -> Result<(), crate::DatabaseError> {
        let error = truncate_cleanup_error(error);
        self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE git_cleanup_jobs
                 SET attempts = attempts + 1, next_attempt_at = ?3,
                     last_attempt_at = ?4, last_error = ?2, updated_at = ?4
                 WHERE id = ?1 AND state = 'pending'",
                params![job_id.as_ref(), error, next_attempt_at, now],
            )?;
            Ok(())
        })
    }

    /// Parks one job for operator intervention; automatic processing stops.
    pub fn mark_manual_attention(
        &self,
        job_id: &GitCleanupJobId,
        error: &str,
        now: i64,
    ) -> Result<(), crate::DatabaseError> {
        let error = truncate_cleanup_error(error);
        self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE git_cleanup_jobs
                 SET state = 'manual_attention', attempts = attempts + 1,
                     last_attempt_at = ?3, last_error = ?2, updated_at = ?3
                 WHERE id = ?1 AND state = 'pending'",
                params![job_id.as_ref(), error, now],
            )?;
            Ok(())
        })
    }

    /// Deletes completed jobs whose retention period has elapsed.
    pub fn purge_completed_before(&self, cutoff: i64) -> Result<usize, crate::DatabaseError> {
        self.pool.with_connection(|connection| {
            let removed = connection.execute(
                "DELETE FROM git_cleanup_jobs WHERE state = 'completed' AND updated_at < ?1",
                params![cutoff],
            )?;
            Ok(removed)
        })
    }
}

/// Inserts cleanup jobs inside an already-open transaction.
///
/// This is the hook cascade deletions use so job registration commits or rolls
/// back atomically with the soft deletes that produced the jobs.
pub(super) fn insert_jobs(
    transaction: &Transaction<'_>,
    jobs: &[GitCleanupJob],
) -> Result<(), rusqlite::Error> {
    for job in jobs {
        transaction.execute(
            "INSERT INTO git_cleanup_jobs (
                 id, project_id, workspace_id, repository_root, checkout_root, branch_name,
                 state, attempts, next_attempt_at, last_attempt_at, last_error,
                 created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                job.id.as_ref(),
                job.project_id.as_ref(),
                job.workspace_id.as_ref(),
                job.repository_root,
                job.checkout_root,
                job.branch_name,
                job.state.database_value(),
                job.attempts,
                job.next_attempt_at,
                job.last_attempt_at,
                job.last_error,
                job.created_at,
                job.updated_at,
            ],
        )?;
    }
    Ok(())
}

/// Reconstructs one persisted cleanup job from its selected columns.
fn map_job_row(row: &Row<'_>) -> Result<GitCleanupJob, crate::DatabaseError> {
    let state = GitCleanupJobState::from_database_value(&row.get::<_, String>("state")?)?;
    Ok(GitCleanupJob {
        id: GitCleanupJobId::new(row.get::<_, String>("id")?),
        project_id: ProjectId::new(row.get::<_, String>("project_id")?),
        workspace_id: WorkspaceId::new(row.get::<_, String>("workspace_id")?),
        repository_root: row.get("repository_root")?,
        checkout_root: row.get("checkout_root")?,
        branch_name: row.get("branch_name")?,
        state,
        attempts: row.get("attempts")?,
        next_attempt_at: row.get("next_attempt_at")?,
        last_attempt_at: row.get("last_attempt_at")?,
        last_error: row.get("last_error")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

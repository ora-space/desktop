mod gated_provisioner;
mod keyed_locks;

#[cfg(test)]
mod tests;

pub(crate) use gated_provisioner::GatedWorktreeProvisioner;
pub(crate) use keyed_locks::{KeyedResourceLocks, SharedLeaseGuard};

use crate::clock::SystemClock;
use ora_application::{
    CleanupJobDisposition, Clock, GitTaskResourceCleaner, RemoveTaskBranchRequest,
    RemoveTaskWorktreeRequest, TaskGitResourceCleaner, legacy_checkout_probe,
    reduce_cleanup_outcomes, validate_cleanup_identity,
};
use ora_db::{
    RepositoryPool, SqliteGitCleanupJobRepository, SqliteWorktreeProvisioningLeaseRepository,
};
use ora_domain::GitCleanupJob;
use ora_logging::{ora_error, ora_info, ora_warn};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::time::Duration;

/// How many jobs one pass pulls per batch, for cross-repository fairness.
const JOB_BATCH_SIZE: usize = 16;
/// Upper bound on repositories cleaned concurrently within one pass.
const MAX_CONCURRENT_REPOSITORIES: usize = 4;
/// Retry attempts before a job parks for manual attention.
const MAX_JOB_ATTEMPTS: i64 = 5;
/// Exponential-ish backoff schedule indexed by the attempt that just failed.
const RETRY_BACKOFF_MS: [i64; 5] = [60_000, 300_000, 1_800_000, 7_200_000, 43_200_000];
/// Idle interval between scan passes when no deletion wakes the worker.
const SCAN_INTERVAL: Duration = Duration::from_secs(60);
/// Retention period for completed jobs before they are purged.
const COMPLETED_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Coalesced wake-up signal shared between deleters and the worker thread.
///
/// The signal only reduces latency; the durable queue in SQLite is the source
/// of truth, so a lost wake-up is always recovered by the periodic scan.
#[derive(Debug, Default)]
struct WakeSignal {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl WakeSignal {
    /// Requests one worker pass; concurrent requests coalesce.
    ///
    /// The flag is a plain boolean, so continuing past a poisoned lock is safe.
    fn notify(&self) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_one();
    }

    /// Waits until notified or until the scan interval elapses.
    fn wait(&self, timeout: Duration) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if !*pending {
            let (guard, _timed_out) = self
                .changed
                .wait_timeout(pending, timeout)
                .unwrap_or_else(PoisonError::into_inner);
            pending = guard;
        }
        *pending = false;
    }
}

/// Shared handle the rest of the backend uses to interact with Git cleanup.
///
/// Deletion paths call [`GitCleanupHandle::notify`] after their transaction
/// committed; worktree consumers acquire shared use leases so physical cleanup
/// waits for in-flight readers instead of pulling checkouts out from under
/// them.
#[derive(Clone, Debug)]
pub(crate) struct GitCleanupHandle {
    wake: Arc<WakeSignal>,
    worktree_use: Arc<KeyedResourceLocks>,
}

impl GitCleanupHandle {
    /// Wakes the worker after a deletion committed cleanup jobs.
    pub(crate) fn notify(&self) {
        self.wake.notify();
    }

    /// Acquires the shared worktree-use lease for one Workspace.
    ///
    /// Every operation that resolves and then reads or mutates a Workspace checkout
    /// must hold this for the duration of the filesystem access.
    pub(crate) fn shared_worktree_use(&self, workspace_id: &str) -> SharedLeaseGuard {
        self.worktree_use.acquire_shared(workspace_id)
    }
}

/// Owns the durable Git cleanup execution for one backend instance.
///
/// The worker consumes pending cleanup jobs and expired provisioning leases,
/// executes the physical Git removals through a [`TaskGitResourceCleaner`],
/// and records the per-job state transitions. Jobs stay `pending` during
/// execution; the in-process single worker is the only executor, so a crash at
/// any point leaves the job replayable on the next start.
pub(crate) struct GitCleanupWorker<Cleaner> {
    pool: RepositoryPool,
    clock: SystemClock,
    cleaner: Cleaner,
    worktree_root: Arc<RwLock<PathBuf>>,
    wake: Arc<WakeSignal>,
    worktree_use: Arc<KeyedResourceLocks>,
    repository_gates: Arc<KeyedResourceLocks>,
}

impl GitCleanupWorker<GitTaskResourceCleaner> {
    /// Builds the production worker over the shared persistence pool.
    pub(crate) fn new(
        pool: RepositoryPool,
        worktree_root: Arc<RwLock<PathBuf>>,
        clock: SystemClock,
    ) -> Self {
        Self::with_cleaner(pool, worktree_root, clock, GitTaskResourceCleaner::new())
    }
}

impl<Cleaner> GitCleanupWorker<Cleaner>
where
    Cleaner: TaskGitResourceCleaner + Send + Sync + 'static,
{
    /// Builds a worker with an injected cleaner (tests use fakes here).
    pub(crate) fn with_cleaner(
        pool: RepositoryPool,
        worktree_root: Arc<RwLock<PathBuf>>,
        clock: SystemClock,
        cleaner: Cleaner,
    ) -> Self {
        Self {
            pool,
            clock,
            cleaner,
            worktree_root,
            wake: Arc::new(WakeSignal::default()),
            worktree_use: KeyedResourceLocks::new(),
            repository_gates: KeyedResourceLocks::new(),
        }
    }

    /// Returns the handle deletion paths and worktree consumers share.
    pub(crate) fn handle(&self) -> GitCleanupHandle {
        GitCleanupHandle {
            wake: Arc::clone(&self.wake),
            worktree_use: Arc::clone(&self.worktree_use),
        }
    }

    /// Returns the repository mutation gate provisioning shares with cleanup.
    pub(crate) fn repository_gates(&self) -> Arc<KeyedResourceLocks> {
        Arc::clone(&self.repository_gates)
    }

    /// Detaches the worker thread: one immediate reconciliation pass, then
    /// notify-or-interval passes until the process exits.
    ///
    /// There is intentionally no graceful drain: an interrupted pass leaves
    /// every unfinished job `pending`, and restart replay is the recovery
    /// mechanism.
    pub(crate) fn spawn(self) -> GitCleanupHandle {
        let handle = self.handle();
        let spawned = std::thread::Builder::new()
            .name("git-cleanup-worker".to_string())
            .spawn(move || {
                loop {
                    self.run_pass();
                    self.wake.wait(SCAN_INTERVAL);
                }
            });
        if let Err(error) = spawned {
            // Without the worker no cleanup runs this process lifetime, but the
            // durable queue keeps every job; the next start replays them.
            ora_error!(
                operation = "git_cleanup",
                error = %error,
                "failed to spawn git cleanup worker thread; cleanup deferred to next start",
            );
        }
        handle
    }

    /// Executes one full pass: lease reclaim, retention purge, then due jobs.
    pub(crate) fn run_pass(&self) {
        let now = self.clock.now_timestamp_millis();
        self.reclaim_expired_leases(now);
        self.purge_completed(now);
        loop {
            let jobs = match SqliteGitCleanupJobRepository::new(self.pool.clone())
                .due_jobs(self.clock.now_timestamp_millis(), JOB_BATCH_SIZE)
            {
                Ok(jobs) => jobs,
                Err(error) => {
                    ora_error!(
                        operation = "git_cleanup",
                        error = %error,
                        "failed to load due git cleanup jobs",
                    );
                    return;
                }
            };
            if jobs.is_empty() {
                return;
            }
            let batch_len = jobs.len();
            self.execute_batch(jobs);
            if batch_len < JOB_BATCH_SIZE {
                return;
            }
        }
    }

    /// Converts expired provisioning leases into pending cleanup jobs.
    fn reclaim_expired_leases(&self, now: i64) {
        match SqliteWorktreeProvisioningLeaseRepository::new(self.pool.clone())
            .reclaim_expired_leases(now)
        {
            Ok(jobs) => {
                for job in &jobs {
                    ora_warn!(
                        operation = "git_cleanup",
                        project_id = %job.project_id,
                        workspace_id = %job.workspace_id,
                        branch_name = %job.branch_name,
                        "expired provisioning lease reclaimed into a cleanup job",
                    );
                }
            }
            Err(error) => {
                ora_error!(
                    operation = "git_cleanup",
                    error = %error,
                    "failed to reclaim expired provisioning leases",
                );
            }
        }
    }

    /// Applies the completed-job retention policy.
    fn purge_completed(&self, now: i64) {
        if let Err(error) = SqliteGitCleanupJobRepository::new(self.pool.clone())
            .purge_completed_before(now - COMPLETED_RETENTION_MS)
        {
            ora_error!(
                operation = "git_cleanup",
                error = %error,
                "failed to purge completed git cleanup jobs",
            );
        }
    }

    /// Runs one batch grouped by repository: groups run concurrently under a
    /// global cap, jobs within a repository run serially.
    fn execute_batch(&self, jobs: Vec<GitCleanupJob>) {
        let mut groups: HashMap<String, Vec<GitCleanupJob>> = HashMap::new();
        for job in jobs {
            groups
                .entry(normalize_repository_key(Path::new(&job.repository_root)))
                .or_default()
                .push(job);
        }
        let mut groups = groups.into_values().collect::<Vec<_>>();
        // A single repository group needs no fan-out; running it inline also
        // keeps thread-scoped tracing subscribers attached in tests.
        if groups.len() == 1 {
            for job in groups.remove(0) {
                self.execute_job(job);
            }
            return;
        }
        let worker_count = MAX_CONCURRENT_REPOSITORIES.min(groups.len());
        let queue = Mutex::new(groups);
        std::thread::scope(|scope| {
            for _ in 0..worker_count {
                scope.spawn(|| {
                    loop {
                        let group = {
                            // The queue is a plain work list; a panicked sibling
                            // poisoning it must not strand the remaining groups.
                            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
                            queue.pop()
                        };
                        let Some(group) = group else { return };
                        for job in group {
                            self.execute_job(job);
                        }
                    }
                });
            }
        });
    }

    /// Executes one job end to end and records its state transition.
    fn execute_job(&self, job: GitCleanupJob) {
        let repository = SqliteGitCleanupJobRepository::new(self.pool.clone());

        if let Err(reason) = validate_cleanup_identity(&job) {
            ora_error!(
                operation = "git_cleanup",
                cleanup_stage = "identity",
                job_id = %job.id,
                project_id = %job.project_id,
                workspace_id = %job.workspace_id,
                branch_name = %job.branch_name,
                attempts = job.attempts,
                error = %reason,
                "git cleanup job failed identity validation; manual attention required",
            );
            self.record(|now| repository.mark_manual_attention(&job.id, &reason, now));
            return;
        }

        // Exclusive use lease: waits for in-flight consumers of this task's
        // checkout; new consumers are already rejected by the soft-deleted rows.
        let _use_lease = self
            .worktree_use
            .acquire_exclusive(job.workspace_id.as_ref());
        // Repository gate: serializes Git mutations per repository so cleanup
        // does not race provisioning on Git's own lock files.
        let _repo_gate = self
            .repository_gates
            .acquire_exclusive(normalize_repository_key(Path::new(&job.repository_root)));

        let legacy_probe = match self.worktree_root.read() {
            // Only derive the legacy probe when the job predates persisted
            // checkout roots; jobs with evidence never touch configuration.
            Ok(root) => {
                (job.checkout_root.is_none()).then(|| legacy_checkout_probe(root.as_path(), &job))
            }
            Err(_poisoned) => None,
        };
        let executed = catch_unwind(AssertUnwindSafe(|| {
            let worktree = self.cleaner.remove_worktree(RemoveTaskWorktreeRequest {
                repository_root: PathBuf::from(&job.repository_root),
                branch_name: job.branch_name.clone(),
                checkout_root: job.checkout_root.clone().map(PathBuf::from),
                legacy_checkout_probe: legacy_probe,
            });
            let branch = self.cleaner.remove_branch(RemoveTaskBranchRequest {
                repository_root: PathBuf::from(&job.repository_root),
                branch_name: job.branch_name.clone(),
            });
            (worktree, branch)
        }));

        let disposition = match &executed {
            Ok((worktree, branch)) => reduce_cleanup_outcomes(worktree, branch),
            Err(panic) => CleanupJobDisposition::Retry {
                error: format!("cleaner panicked: {}", panic_message(panic)),
            },
        };

        match disposition {
            CleanupJobDisposition::Completed => {
                ora_info!(
                    operation = "git_cleanup",
                    job_id = %job.id,
                    project_id = %job.project_id,
                    workspace_id = %job.workspace_id,
                    branch_name = %job.branch_name,
                    attempts = job.attempts,
                    "git cleanup job completed",
                );
                self.record(|now| repository.mark_completed(&job.id, now));
            }
            CleanupJobDisposition::Retry { error } => {
                let failed_attempts = job.attempts + 1;
                if failed_attempts >= MAX_JOB_ATTEMPTS {
                    ora_error!(
                        operation = "git_cleanup",
                        cleanup_stage = "task_resources",
                        job_id = %job.id,
                        project_id = %job.project_id,
                        workspace_id = %job.workspace_id,
                        branch_name = %job.branch_name,
                        attempts = job.attempts,
                        error = %error,
                        "git cleanup job exhausted retries; manual attention required",
                    );
                    self.record(|now| repository.mark_manual_attention(&job.id, &error, now));
                } else {
                    let backoff_index =
                        (failed_attempts as usize - 1).min(RETRY_BACKOFF_MS.len() - 1);
                    let backoff = RETRY_BACKOFF_MS[backoff_index];
                    ora_warn!(
                        operation = "git_cleanup",
                        cleanup_stage = "task_resources",
                        job_id = %job.id,
                        project_id = %job.project_id,
                        workspace_id = %job.workspace_id,
                        branch_name = %job.branch_name,
                        attempts = job.attempts,
                        error = %error,
                        "git cleanup job failed; retry scheduled",
                    );
                    self.record(|now| {
                        repository.record_retryable_failure(&job.id, &error, now + backoff, now)
                    });
                }
            }
            CleanupJobDisposition::ManualAttention { reason } => {
                ora_error!(
                    operation = "git_cleanup",
                    cleanup_stage = "task_resources",
                    job_id = %job.id,
                    project_id = %job.project_id,
                    workspace_id = %job.workspace_id,
                    branch_name = %job.branch_name,
                    attempts = job.attempts,
                    error = %reason,
                    "git cleanup job requires manual attention",
                );
                self.record(|now| repository.mark_manual_attention(&job.id, &reason, now));
            }
        }
    }

    /// Persists one state transition, logging instead of failing the pass.
    fn record(&self, transition: impl FnOnce(i64) -> Result<(), ora_db::DatabaseError>) {
        if let Err(error) = transition(self.clock.now_timestamp_millis()) {
            ora_error!(
                operation = "git_cleanup",
                error = %error,
                "failed to persist git cleanup job state transition",
            );
        }
    }
}

/// Normalizes a repository root into the key shared by gates and grouping.
///
/// Canonicalization resolves symlinks and case quirks when the path exists;
/// missing paths fall back to their lexical form, which still groups all jobs
/// of one recorded repository together.
pub(crate) fn normalize_repository_key(repository_root: &Path) -> String {
    std::fs::canonicalize(repository_root)
        .unwrap_or_else(|_| repository_root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Extracts a human-readable message from a panic payload.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

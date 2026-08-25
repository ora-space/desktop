use crate::RepositoryError;
use ora_domain::{Task, Worktree, WorktreeProvisioningLease, WorktreeProvisioningLeaseId};
use std::sync::mpsc;
use std::time::Duration;

/// How long one provisioning lease stays valid without renewal.
pub const PROVISIONING_LEASE_DURATION_MS: i64 = 10 * 60 * 1000;
/// How often the renewal guard extends a live lease during slow Git work.
pub const PROVISIONING_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(60);

/// Persists write-ahead provisioning leases for in-flight worktree creation.
///
/// Implementations must make `release_to_cleanup` atomic (lease removal and
/// cleanup-job insertion in one transaction) so an abandoned provisioning can
/// never lose its Git resources between the two steps. `Clone + Send` is
/// required because the renewal guard extends the lease from a helper thread
/// while the creating thread runs Git.
pub trait WorktreeProvisioningLeaseStore: Clone + Send + 'static {
    /// Persists a new lease before any Git mutation starts.
    fn create_lease(&self, lease: &WorktreeProvisioningLease) -> Result<(), RepositoryError>;

    /// Extends one live lease; returns false when the lease no longer exists.
    fn renew_lease(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
        lease_expires_at: i64,
        now: i64,
    ) -> Result<bool, RepositoryError>;

    /// Converts one lease into an immediately pending cleanup job atomically.
    ///
    /// Used when creation fails after Git provisioning: the durable cleanup
    /// path removes both the worktree and the branch, which ad-hoc inline
    /// compensation could only partially cover.
    fn release_to_cleanup(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
        now: i64,
    ) -> Result<(), RepositoryError>;
}

/// Reports whether one task-creation commit went through or lost to a
/// concurrent project deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCommitOutcome {
    Committed,
    ProjectNotVisible,
}

/// Commits task creation atomically against concurrent project deletion.
///
/// Implementations must re-validate project visibility, persist the task (and
/// worktree) rows, and remove the provisioning lease inside one transaction:
/// either a project cascade sees the committed rows and registers cleanup jobs
/// for them, or the commit reports `ProjectNotVisible` and the caller releases
/// the lease to cleanup.
pub trait TaskWorkspaceCommit {
    /// Atomically persists a worktree-backed task and releases its lease.
    fn commit_worktree_task(
        &self,
        task: &Task,
        worktree: &Worktree,
        lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError>;
}

/// Keeps one provisioning lease alive from a helper thread until dropped.
///
/// Renewal is what distinguishes "slow Git work" from "owner died": the lease
/// worker only reclaims leases that expired without renewal, so a legitimate
/// multi-minute `git worktree add` is never treated as abandoned while its
/// creating thread is alive.
pub struct ProvisioningLeaseRenewal {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProvisioningLeaseRenewal {
    /// Spawns the renewal thread for one lease.
    pub fn spawn<Store, Now>(
        store: Store,
        lease_id: WorktreeProvisioningLeaseId,
        now_timestamp_millis: Now,
    ) -> Self
    where
        Store: WorktreeProvisioningLeaseStore,
        Now: Fn() -> i64 + Send + 'static,
    {
        let (stop, stopped) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("worktree-lease-renewal".to_string())
            .spawn(move || {
                loop {
                    match stopped.recv_timeout(PROVISIONING_LEASE_RENEW_INTERVAL) {
                        // Guard dropped (or sender vanished): stop renewing.
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let now = now_timestamp_millis();
                            // A failed or missing renewal is not fatal here: the
                            // worst case is the lease expiring and the cleanup
                            // worker reclaiming resources the commit also owns,
                            // which the idempotent cleanup absorbs.
                            let _ = store.renew_lease(
                                &lease_id,
                                now + PROVISIONING_LEASE_DURATION_MS,
                                now,
                            );
                        }
                    }
                }
            });
        // A failed spawn only disables renewal: the lease then runs on its
        // initial duration, which still covers ordinary provisioning times.
        Self {
            stop: Some(stop),
            thread: spawned.ok(),
        }
    }
}

impl Drop for ProvisioningLeaseRenewal {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

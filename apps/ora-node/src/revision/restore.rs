//! Restoring a session's prior Revision into its checkout before the agent starts (Node delivery
//! push ADR D5, D6; Node clone ADR D9).
//!
//! The Node fetches the prior delivery ref from the Git Server into a transport repository that
//! borrows the checkout's objects, negotiating from the clone's base commit so only what the
//! checkout lacks is sent, and checks that it is the prior final commit. The workload user then
//! takes the objects from the transport repository into the checkout and moves the Workspace
//! branch there. Everything runs on a blocking thread; the transport repository is removed when
//! the restore ends, whatever the outcome.

mod git;

use super::snapshot::DeliveryGit;
use crate::git_server::{GitServerAccess, GitServerFailure, TimeLimit, TransportRepository};
use crate::session::{
    PriorRevisionRestore, RestoreCancel, RestoreFailure, RestoreRequest, Restored,
};
use git::GitRestore;
use gitlancer::GitRunner;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Kind of the transport repositories restores use.
const TRANSPORT_KIND: &str = "restore";
/// The fetch from the Git Server and the workload's import share this deadline, retries included
/// (Node delivery push ADR D5); `network_timeout_ms` does not apply.
const RESTORE_LIMIT: Duration = Duration::from_secs(5 * 60);

/// The workload user's Git and the Node identity's Git a restore runs.
type RestoreGit<R, N> = (DeliveryGit<R>, N);

/// The production restore: Git Server access through `N` as the Node, checkout Git through `R`
/// with delivery's policy as the workload user.
pub(crate) struct RevisionRestorer<R: Send + 'static, N: Send + 'static> {
    /// One restore's Git at a time: a host-backed runner drives its own runtime, which concurrent
    /// blocking threads must not share.
    /// Taken out only when the restorer is dropped.
    git: Arc<Mutex<Option<RestoreGit<R, N>>>>,
    access: GitServerAccess,
    /// The clone repository root, whose transport directory holds the restore's repository.
    repository_root: PathBuf,
    /// The workload user restore Git runs as, when the deployment separates it from the Node.
    owner: Option<u32>,
    /// Ends the waits between Git Server attempts when the Node stops.
    shutdown: crate::Shutdown,
    limit: Duration,
}

impl<R: Send + 'static, N: Send + 'static> RevisionRestorer<R, N> {
    pub(crate) fn new(
        git: DeliveryGit<R>,
        node: N,
        access: GitServerAccess,
        repository_root: PathBuf,
        owner: Option<u32>,
        shutdown: crate::Shutdown,
    ) -> Self {
        Self {
            git: Arc::new(Mutex::new(Some((git, node)))),
            access,
            repository_root,
            owner,
            shutdown,
            limit: RESTORE_LIMIT,
        }
    }

    /// The same restorer with a shorter deadline, for tests that run it out.
    #[cfg(test)]
    pub(super) fn within(mut self, limit: Duration) -> Self {
        self.limit = limit;
        self
    }
}

impl<R: Send + 'static, N: Send + 'static> Drop for RevisionRestorer<R, N> {
    /// Host-backed runners own synchronous runtimes, and the last session handle may be released
    /// on an async worker; those runtimes are retired on a joined thread that may block.
    fn drop(&mut self) {
        let git = self
            .git
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(git) = git {
            let _ = std::thread::spawn(move || drop(git)).join();
        }
    }
}

impl<R, N> PriorRevisionRestore for RevisionRestorer<R, N>
where
    R: GitRunner + Send + 'static,
    N: GitRunner + Send + 'static,
{
    /// Fetches, verifies and checks out the prior final commit on a blocking thread, which
    /// starts no further Git once the Node stops or the session is cancelled.
    async fn restore(
        &self,
        request: RestoreRequest,
        cancel: RestoreCancel,
    ) -> Result<Restored, RestoreFailure> {
        let git = Arc::clone(&self.git);
        let deadline = Instant::now() + self.limit;
        let access = self.access.limited(TimeLimit::Until(deadline));
        let root = self.repository_root.clone();
        let owner = self.owner;
        let shutdown = self.shutdown.clone();
        tokio::task::spawn_blocking(move || {
            let stopping = || shutdown.requested() || cancel.requested();
            let git = git.lock().unwrap_or_else(PoisonError::into_inner);
            let Some((workload, node)) = git.as_ref() else {
                return Err(RestoreFailure::Unavailable);
            };
            let execution = request.execution.as_str();
            let transport =
                TransportRepository::create(node, &root, TRANSPORT_KIND, &request.execution)
                    .and_then(|transport| {
                        transport.borrow_objects(&request.checkout.join(".git"))?;
                        transport.negotiate_from(node, &[&request.base_commit])?;
                        Ok(transport)
                    })
                    .map_err(|error| {
                        ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore transport repository could not be prepared");
                        RestoreFailure::Unavailable
                    })?;
            let prior = &request.prior;
            access
                .fetch(
                    node,
                    &transport,
                    &prior.git_repository_url,
                    &request.execution,
                    &[format!("{}:refs/ora/prior", prior.delivery_ref.as_str())],
                    &stopping,
                )
                .map_err(|failure| {
                    ora_logging::ora_warn!(execution_id = %execution, failure = ?failure, "prior delivery ref was not fetched");
                    match failure {
                        // The Git Server answers missing and unreadable refs the same way: the
                        // prior Revision's Git content is permanently out of reach.
                        GitServerFailure::Denied | GitServerFailure::MissingRef => {
                            RestoreFailure::BaseUnavailable
                        }
                        GitServerFailure::Unavailable | GitServerFailure::Stopped => {
                            RestoreFailure::Unavailable
                        }
                    }
                })?;
            // A delivery ref only ever gets created, so another commit is a defect; it stays
            // retryable instead of making Cloud give up on the Revision.
            match transport.commit_of(node, "refs/ora/prior") {
                Ok(Some(commit)) if commit == prior.final_commit => {}
                Ok(_) | Err(_) => {
                    ora_logging::ora_warn!(execution_id = %execution, "prior delivery ref does not hold the prior final commit");
                    return Err(RestoreFailure::Unavailable);
                }
            }
            if stopping() {
                return Err(RestoreFailure::Unavailable);
            }
            transport.hand_over(owner).map_err(|error| {
                ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore transport repository could not be handed over");
                RestoreFailure::Unavailable
            })?;
            workload.restore(&GitRestore {
                checkout: &request.checkout,
                transport: transport.path(),
                prior,
                branch: &request.branch,
                base_commit: &request.base_commit,
                deadline,
                stopping: &stopping,
            })
        })
        .await
        .unwrap_or(Err(RestoreFailure::Unavailable))
    }
}

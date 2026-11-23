//! Restoring a session's prior Revision into its checkout before the agent starts (Node delivery
//! push ADR D5, D6; Node clone ADR D9).
//!
//! The Node fetches the prior delivery ref from the Git Server into a transport repository that
//! borrows the checkout's objects, negotiating from the clone's base commit so only what the
//! checkout lacks is sent, and checks that it is the prior final commit. The workload user then
//! takes the objects from the transport repository into the checkout and moves the Workspace
//! branch there. Everything runs on a blocking thread; the transport repository is removed once
//! the restore ends and every original command of it has confirmed its stop, whatever the
//! outcome. One absolute deadline, owned by the deployment, bounds the Git Server accesses and
//! every workload Git command at both identities; neither a retry nor a wait grants more time, and
//! unknown original stop evidence retains the inputs across restart without rerunning the restore.

mod config;
mod git;
mod runner;

pub use config::RestoreConfig;
pub(crate) use runner::RestoreGitRunner;

use super::snapshot::DeliveryGit;
use crate::git_server::{GitServerAccess, GitServerFailure, TimeLimit, TransportRepository};
use crate::session::{
    PriorRevisionRestore, RestoreCancel, RestoreFailure, RestoreRequest, Restored,
};
use git::GitRestore;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Kind of the transport repositories restores use.
pub(crate) const TRANSPORT_KIND: &str = "restore";

/// One restore's execution timing; cleanup keeps its independent managed-process budget.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RestorePolicy {
    /// One absolute budget shared by the Git Server fetch and all workload Git steps.
    pub(crate) deadline: Duration,
    /// One bound for original stop confirmation, counted from the first stop request.
    pub(crate) stop_confirmation: Duration,
}

/// The workload user's Git and the Node identity's Git a restore runs.
type RestoreGit<R, N> = (DeliveryGit<R>, N);

/// The production restore: Git Server access through `N` as the Node, checkout Git through `R`
/// with delivery's policy as the workload user.
pub(crate) struct RevisionRestorer<R: Send + 'static, N: Send + 'static> {
    /// One restore's Git at a time: the production runner drives its own runtime, which concurrent
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
    policy: RestorePolicy,
    /// Production shares its stop authority with crash recovery and late execution queries.
    stops: Option<crate::managed::RestoreStops>,
}

impl<R: Send + 'static, N: Send + 'static> RevisionRestorer<R, N> {
    /// Production shares its stop authority with crash recovery and late execution queries.
    pub(crate) fn with_stops(mut self, stops: crate::managed::RestoreStops) -> Self {
        self.stops = Some(stops);
        self
    }

    /// Composes one restorer for the Node's lifetime; only the deployment chooses its budgets.
    pub(crate) fn new(
        git: DeliveryGit<R>,
        node: N,
        access: GitServerAccess,
        repository_root: PathBuf,
        owner: Option<u32>,
        shutdown: crate::Shutdown,
        policy: RestorePolicy,
    ) -> Self {
        Self {
            git: Arc::new(Mutex::new(Some((git, node)))),
            access,
            repository_root,
            owner,
            shutdown,
            policy,
            stops: None,
        }
    }

    /// The same restorer with a shorter deadline, for tests that run it out.
    #[cfg(test)]
    pub(super) fn within(mut self, limit: Duration) -> Self {
        self.policy.deadline = limit;
        self
    }
}

impl<R: Send + 'static, N: Send + 'static> Drop for RevisionRestorer<R, N> {
    /// The production runners own synchronous runtimes, and the last session handle may be released
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
    R: RestoreGitRunner + Send + 'static,
    N: RestoreGitRunner + Send + 'static,
{
    /// Anchors the stop clock before cancellation can reach the command boundary.
    fn request_stop(&self, execution: &ora_node_protocol::ExecutionId) {
        if let Some(stops) = &self.stops
            && let Err(error) = stops.request_session(execution)
        {
            ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore stop origin could not be persisted; stopping still has priority");
        }
    }

    /// Waits for original stop proof, then removes the transport repository it guarded.
    async fn confirm_stop(&self, execution: &ora_node_protocol::ExecutionId) -> bool {
        let confirmed = match &self.stops {
            Some(stops) => stops.wait(execution).await,
            None => true,
        };
        if confirmed {
            crate::git_server::remove_transport(&TransportRepository::location(
                &self.repository_root,
                TRANSPORT_KIND,
                execution,
            ));
        }
        confirmed
    }

    /// Fetches, verifies and checks out the prior final commit on a blocking thread, which
    /// starts no further Git once the Node stops, the session is cancelled or the deadline ends.
    async fn restore(
        &self,
        request: RestoreRequest,
        cancel: RestoreCancel,
    ) -> Result<Restored, RestoreFailure> {
        if self.policy.deadline.is_zero() {
            return Err(RestoreFailure::Unavailable);
        }
        let deadline = Instant::now()
            .checked_add(self.policy.deadline)
            .ok_or(RestoreFailure::Unavailable)?;
        let git = Arc::clone(&self.git);
        let access = self.access.limited(TimeLimit::Until(deadline));
        let root = self.repository_root.clone();
        let owner = self.owner;
        let shutdown = self.shutdown.clone();
        let stops = self.stops.clone();
        tokio::task::spawn_blocking(move || {
            let stopping = || shutdown.requested() || cancel.requested();
            let mut git = git.lock().unwrap_or_else(PoisonError::into_inner);
            let Some((workload, node)) = git.as_mut() else {
                return Err(RestoreFailure::Unavailable);
            };
            // Every restore command is recorded as an attempt before it is dispatched, whether the
            // Node or the workload user runs it, and both share the session's stop cancellation.
            workload.bind_restore(&request.execution, cancel.clone());
            node.bind_restore(&request.execution, cancel.clone());
            let commands = runner::RestoreCommands {
                runner: &*node,
                deadline,
            };
            let execution = request.execution.as_str();
            let transport = TransportRepository::prepare(&root, TRANSPORT_KIND, &request.execution)
                .map_err(|error| {
                    ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore transport repository could not be prepared");
                    RestoreFailure::Unavailable
                })?;
            let restored = (|| -> Result<Restored, RestoreFailure> {
                transport.init(&commands).map_err(|error| {
                    ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore transport repository could not be created");
                    RestoreFailure::Unavailable
                })?;
                transport
                    .borrow_objects(&request.checkout.join(".git"))
                    .map_err(|error| {
                        ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore transport repository could not borrow the checkout's objects");
                        RestoreFailure::Unavailable
                    })?;
                // Local staging between Git commands takes the same admission fence as Git.
                commands.check_admission().map_err(|error| {
                    ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore admission was refused");
                    RestoreFailure::Unavailable
                })?;
                transport
                    .negotiate_from(&commands, &[&request.base_commit])
                    .map_err(|error| {
                        ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore transport repository could not negotiate its base");
                        RestoreFailure::Unavailable
                    })?;
                let prior = &request.prior;
                access
                    .fetch(
                        &commands,
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
                            // A 401 only says the Node's own git-access material is wrong or
                            // expired; it must not mark the prior Revision unresumable, so a later
                            // session may try again once the deployment is fixed.
                            GitServerFailure::Unauthenticated
                            | GitServerFailure::Unavailable
                            | GitServerFailure::Stopped => RestoreFailure::Unavailable,
                        }
                    })?;
                // A delivery ref only ever gets created, so another commit is a defect; it stays
                // retryable instead of making Cloud give up on the Revision.
                match transport.commit_of(&commands, "refs/ora/prior") {
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
            })();
            // The workload user reads objects out of the transport repository, so it stays until
            // every original command of this restore has proven that it stopped.
            if stops
                .as_ref()
                .is_some_and(|stops| stops.retains_inputs(&request.execution))
            {
                transport.keep();
            }
            restored
        })
        .await
        .unwrap_or(Err(RestoreFailure::Unavailable))
    }
}

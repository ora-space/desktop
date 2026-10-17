//! Pushing a frozen delivery's final commit to the Git Server (Node delivery push ADR D1, D2, D4;
//! Node clone ADR D8).
//!
//! The push runs as the Node from a transport repository that borrows the checkout's objects. It
//! first creates the delivery ref, which decides the Revision, then fast-forwards the Workspace
//! branch with the old value the Node's ledger knows; the second push never fails the delivery.
//! A replay after a restart pushes the same commit, which the Git Server reports as up to date.
use crate::git_server::{
    GitServerAccess, GitServerFailure, Lease, PushReport, RefUpdate, TimeLimit, TransportRepository,
};
use gitlancer::GitRunner;
use ora_node_protocol::*;
use std::path::PathBuf;
use std::time::Duration;

/// Kind of the transport repositories deliveries use.
const TRANSPORT_KIND: &str = "delivery";
/// How long one push attempt may take (Node delivery push ADR D4): three attempts and the pauses
/// between them stay within Cloud's two-hour window for giving up on a delivery. It is
/// independent of `network_timeout_ms`, which bounds clone and `ora-git` commands.
const PUSH_ATTEMPT_LIMIT: Duration = Duration::from_secs(30 * 60);

/// Everything one delivery's push needs, resolved by the worker from the ledger.
#[derive(Clone, Debug)]
pub(crate) struct PushJob {
    pub(crate) execution: ExecutionId,
    pub(crate) spec: DeliverRevisionSpec,
    pub(crate) checkout: PathBuf,
    pub(crate) final_commit: CommitId,
    /// The ledger's last known value of the Workspace branch, the old value the fast-forward
    /// declares.
    pub(crate) workspace_head: Option<CommitId>,
    pub(crate) repository_root: PathBuf,
}

/// How a push ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pushed {
    /// The delivery ref holds the final commit; `workspace_head` is the branch value the ledger
    /// should now record, when the fast-forward established one.
    Delivered {
        workspace_head: Option<CommitId>,
    },
    Failed(RevisionFailureCode),
    /// The Node is stopping; the delivery resumes after restart with the same plan.
    Interrupted,
}

/// Pushes the final commit to the delivery ref, then fast-forwards the Workspace branch.
pub(crate) fn push<N: GitRunner>(
    node: &N,
    access: &GitServerAccess,
    job: &PushJob,
    stopping: &dyn Fn() -> bool,
) -> Pushed {
    let access = &access.limited(TimeLimit::PerAttempt(PUSH_ATTEMPT_LIMIT));
    let execution = job.execution.as_str();
    let transport = match TransportRepository::create(
        node,
        &job.repository_root,
        TRANSPORT_KIND,
        &job.execution,
    )
    .and_then(|transport| {
        transport.borrow_objects(&job.checkout.join(".git"))?;
        Ok(transport)
    }) {
        Ok(transport) => transport,
        Err(error) => {
            ora_logging::ora_warn!(execution_id = %execution, error = %error, "delivery transport repository could not be prepared");
            return Pushed::Failed(RevisionFailureCode::SnapshotFailed);
        }
    };
    let delivered = access.push(
        node,
        &transport,
        &job.spec.git_repository_url,
        &job.execution,
        &RefUpdate {
            commit: &job.final_commit,
            destination: job.spec.delivery_ref.as_str(),
            lease: Lease::Unchecked,
        },
        stopping,
    );
    match delivered {
        Ok(PushReport::Updated | PushReport::UpToDate) => {}
        // An existing delivery ref at another commit is refused: only this execution creates it.
        Ok(PushReport::Rejected | PushReport::Stale)
        | Err(GitServerFailure::Denied | GitServerFailure::MissingRef) => {
            ora_logging::ora_warn!(execution_id = %execution, "Git Server refused the delivery ref");
            return Pushed::Failed(RevisionFailureCode::PushRejected);
        }
        Err(GitServerFailure::Unavailable) => {
            ora_logging::ora_warn!(execution_id = %execution, "delivery ref push failed after its attempts");
            return Pushed::Failed(RevisionFailureCode::PushFailed);
        }
        Err(GitServerFailure::Stopped) => return Pushed::Interrupted,
    }
    let workspace_head = fast_forward(node, access, &transport, job, stopping);
    Pushed::Delivered { workspace_head }
}

/// Fast-forwards the Workspace branch to the final commit; every failure is only logged.
fn fast_forward<N: GitRunner>(
    node: &N,
    access: &GitServerAccess,
    transport: &TransportRepository,
    job: &PushJob,
    stopping: &dyn Fn() -> bool,
) -> Option<CommitId> {
    let execution = job.execution.as_str();
    let destination = job.spec.workspace_ref.as_str();
    let lease = job.workspace_head.clone().map_or(Lease::Absent, Lease::At);
    let pushed = access.push(
        node,
        transport,
        &job.spec.git_repository_url,
        &job.execution,
        &RefUpdate {
            commit: &job.final_commit,
            destination,
            lease,
        },
        stopping,
    );
    match pushed {
        Ok(PushReport::Updated | PushReport::UpToDate) => Some(job.final_commit.clone()),
        Ok(PushReport::Stale) => {
            // The branch moved without this checkout knowing; a value equal to the final commit
            // is a replay whose ledger update was lost.
            let current = access
                .remote_refs(
                    node,
                    transport,
                    &job.spec.git_repository_url,
                    &job.execution,
                    &[destination.to_owned()],
                    stopping,
                )
                .ok()
                .and_then(|refs| {
                    refs.into_iter()
                        .find(|(name, _)| name == destination)
                        .map(|(_, commit)| commit)
                });
            if current.as_ref() == Some(&job.final_commit) {
                return current;
            }
            ora_logging::ora_info!(execution_id = %execution, "Workspace branch moved since this checkout last saw it; left unchanged");
            None
        }
        Ok(PushReport::Rejected) | Err(GitServerFailure::Denied | GitServerFailure::MissingRef) => {
            ora_logging::ora_info!(execution_id = %execution, "Workspace branch was not fast-forwarded to the final commit");
            None
        }
        Err(GitServerFailure::Unavailable | GitServerFailure::Stopped) => {
            ora_logging::ora_info!(execution_id = %execution, "Workspace branch fast-forward could not reach the Git Server");
            None
        }
    }
}

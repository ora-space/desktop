//! The workload half of a restore (Node delivery push ADR D5 steps 4–6, Node clone ADR D9), run
//! with delivery's hardened policy.
//!
//! Every command goes through [`DeliveryGit`], so restore shares delivery's runner (host-managed,
//! as the workload user) and its fixed configuration (no hooks, no fsmonitor, no signing) on top
//! of clone's environment and protocol policy. Only the Workspace branch, the index and the
//! worktree change; `refs/remotes/ora/**` and the repository configuration stay as they are.
use super::super::snapshot::DeliveryGit;
use crate::session::{RestoreFailure, Restored};
use gitlancer::{GitExecError, GitRunner};
use ora_node_protocol::{CommitId, PriorRevision};
use std::path::Path;
use std::time::Instant;

/// Background maintenance a fetch could start in the checkout; nothing may outlive the restore.
const QUIET_FETCH: [&str; 4] = ["-c", "gc.auto=0", "-c", "maintenance.auto=false"];

/// The transport repository holding the verified prior final commit, and where it goes.
pub(crate) struct GitRestore<'a> {
    pub(crate) checkout: &'a Path,
    /// Owned by the workload user and free of credentials; it holds `refs/ora/prior`.
    pub(crate) transport: &'a Path,
    pub(crate) prior: &'a PriorRevision,
    /// The Workspace branch short name the clone checked out.
    pub(crate) branch: &'a str,
    /// The clone's base commit: the snapshot the prior Revision's base is judged against.
    pub(crate) base_commit: &'a CommitId,
    /// The import shares the fetch's deadline.
    pub(crate) deadline: Instant,
    /// Whether the session is ending; no Git starts once it is.
    pub(crate) stopping: &'a dyn Fn() -> bool,
}

impl<R: GitRunner> DeliveryGit<R> {
    /// Takes the prior final commit into the checkout, moves the Workspace branch there, and
    /// reports whether the prior base is in the snapshot's history. Blocking: callers keep it off
    /// the async workers.
    pub(crate) fn restore(&self, request: &GitRestore<'_>) -> Result<Restored, RestoreFailure> {
        let checkout = request.checkout;
        let final_commit = &request.prior.final_commit;
        let transport = request
            .transport
            .to_str()
            .ok_or(RestoreFailure::Unavailable)?;
        let mut fetch = QUIET_FETCH.to_vec();
        // The transport repository is the file transport, which clone's protocol policy otherwise
        // forbids; this is the only command that may read it.
        fetch.extend([
            "-c",
            "protocol.file.allow=always",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--",
            transport,
            "refs/ora/prior",
        ]);
        let left = request.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || (request.stopping)() {
            return Err(RestoreFailure::Unavailable);
        }
        self.run(self.command(checkout, &fetch, &[]).with_time_limit(left))
            .map_err(failed)?;
        let peeled = format!("{}^{{commit}}", final_commit.as_str());
        let fetched = self
            .output(checkout, &["rev-parse", "--verify", "-q", &peeled], &[])
            .map_err(failed)?;
        if fetched != final_commit.as_str() || (request.stopping)() {
            return Err(RestoreFailure::Unavailable);
        }
        self.output(
            checkout,
            &[
                "checkout",
                "-q",
                "--force",
                "-B",
                request.branch,
                final_commit.as_str(),
            ],
            &[],
        )
        .map_err(failed)?;
        Ok(self.divergence(checkout, request))
    }

    /// Whether the prior base commit is in the history of this checkout's snapshot; when it is
    /// not (or the checkout lacks it), the external repository's history was rewritten. A runner
    /// failure here only loses the note, so it is logged and treated as no divergence.
    fn divergence(&self, checkout: &Path, request: &GitRestore<'_>) -> Restored {
        let base = &request.prior.base_commit;
        match self.output(
            checkout,
            &[
                "merge-base",
                "--is-ancestor",
                base.as_str(),
                request.base_commit.as_str(),
            ],
            &[],
        ) {
            Ok(_) => Restored::OnSnapshotHistory,
            Err(GitExecError::NonZeroExit { .. }) => Restored::Diverged {
                final_commit: request.prior.final_commit.clone(),
                base_commit: base.clone(),
                branch: request.branch.to_owned(),
            },
            Err(error) => {
                ora_logging::ora_warn!(error = %error, "snapshot history of the restored Revision could not be checked");
                Restored::OnSnapshotHistory
            }
        }
    }
}

/// Keeps Git's diagnostics in the Node log while the session reports only the failure code.
fn failed(error: GitExecError) -> RestoreFailure {
    ora_logging::ora_warn!(error = %error, "prior Revision restore Git step failed");
    RestoreFailure::Unavailable
}

//! Restoring a session's prior Revision into its checkout before the agent starts (Node restore
//! ADR D2-D4).
//!
//! The restorer asks the Controller for a read grant, downloads the bundle into a Node-private
//! directory under the Node home (`revision-restores/<sha256(execution)>/`, directories `0700`,
//! the file `0600`), verifies its size and SHA-256 against the session input, and then runs the
//! Git steps with delivery's hardened policy on a blocking thread. The grant wait and the download
//! and all Git steps share one absolute deadline. The directory is removed when the restore ends, and
//! the whole root is cleared when the Node starts, since no restore survives a restart.

mod bundle;
mod config;
mod download;
mod git;
mod grants;
mod runner;

pub use config::RestoreConfig;
pub(crate) use download::{HttpDownloader, ObjectDownloader};
pub(crate) use grants::DownloadGrants;
pub(crate) use runner::RestoreGitRunner;

use super::snapshot::DeliveryGit;
use crate::session::{PriorRevisionRestore, RestoreFailure, RestoreRequest, Restored};
use download::{DownloadJob, download};
use git::GitRestore;
use ora_node_protocol::NodeId;
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Directory under the Node home holding one private directory per running restore.
pub(crate) const RESTORE_ROOT: &str = "revision-restores";
/// File name of the downloaded bundle inside a restore's directory.
const BUNDLE_FILE: &str = "prior.bundle";

/// One restore's execution timing; cleanup retains its separate managed-process budget.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RestorePolicy {
    /// Bounds download and all Git steps together; none reset this budget.
    pub(crate) deadline: Duration,
    /// Network failures and 5xx answers retried before the restore fails.
    pub(crate) retries: u32,
    /// How often an unanswered grant request is repeated while waiting.
    pub(crate) resend: Duration,
    /// Pause before retrying a failed download.
    pub(crate) backoff: Duration,
    /// A grant expiring sooner than this is treated as already expired.
    pub(crate) expiry_margin: Duration,
}

impl RestorePolicy {
    pub(crate) const DEFAULT: Self = Self {
        deadline: Duration::from_millis(config::DEFAULT_DEADLINE_MS),
        retries: 3,
        resend: Duration::from_secs(/*secs*/ 2),
        backoff: Duration::from_millis(/*millis*/ 500),
        expiry_margin: Duration::from_secs(/*secs*/ 5),
    };
}

/// The production restore: download through `D`, Git through `R` with delivery's policy.
pub(crate) struct RevisionRestorer<R, D> {
    /// One restore's Git at a time: a host-backed runner drives its own runtime, which concurrent
    /// blocking threads must not share.
    git: Arc<Mutex<DeliveryGit<R>>>,
    downloader: D,
    grants: DownloadGrants,
    node: NodeId,
    root: PathBuf,
    /// The workload user restore Git runs as, when the deployment separates it from the Node.
    owner: Option<u32>,
    policy: RestorePolicy,
}

impl<R, D> RevisionRestorer<R, D> {
    pub(crate) fn new(
        git: DeliveryGit<R>,
        downloader: D,
        grants: DownloadGrants,
        node: NodeId,
        root: PathBuf,
        owner: Option<u32>,
        policy: RestorePolicy,
    ) -> Self {
        Self {
            git: Arc::new(Mutex::new(git)),
            downloader,
            grants,
            node,
            root,
            owner,
            policy,
        }
    }
}

impl<R, D> PriorRevisionRestore for RevisionRestorer<R, D>
where
    R: RestoreGitRunner + Send + 'static,
    D: ObjectDownloader,
{
    /// Shares one cutoff across download, waiting for Git and every supervised Git command.
    async fn restore(&self, request: RestoreRequest) -> Result<Restored, RestoreFailure> {
        if self.policy.deadline.is_zero() {
            return Err(RestoreFailure::Unavailable);
        }
        let scratch = ora_utils::hash::sha256_hex(request.execution.as_str().as_bytes());
        let directory = PrivateDirectory::create(&self.root.join(&scratch)).map_err(|error| {
            ora_logging::ora_warn!(execution_id = %request.execution.as_str(), error = %error, "restore directory could not be created");
            RestoreFailure::Unavailable
        })?;
        let job = DownloadJob {
            operation: request.operation.clone(),
            execution: request.execution.clone(),
            node_id: self.node.clone(),
            bundle: request.prior.bundle.clone(),
            destination: directory.0.join(BUNDLE_FILE),
        };
        // The bundle path's approved clock starts immediately before the first grant exchange.
        let deadline = Instant::now()
            .checked_add(self.policy.deadline)
            .ok_or(RestoreFailure::Unavailable)?;
        let downloaded = tokio::time::timeout_at(
            deadline.into(),
            download(&self.downloader, &self.grants, &job, self.policy, deadline),
        )
        .await;
        match downloaded {
            Ok(Ok(())) => {}
            Ok(Err(failure)) => {
                ora_logging::ora_warn!(execution_id = %request.execution.as_str(), failure = %failure, "prior bundle was not downloaded");
                return Err(RestoreFailure::Unavailable);
            }
            Err(_elapsed) => {
                ora_logging::ora_warn!(execution_id = %request.execution.as_str(), "prior bundle was not downloaded before the restore deadline");
                return Err(RestoreFailure::Unavailable);
            }
        }
        let git = tokio::time::timeout_at(deadline.into(), Arc::clone(&self.git).lock_owned())
            .await
            .map_err(|_| RestoreFailure::Unavailable)?;
        let owner = self.owner;
        let bundle = job.destination;
        let dispatcher = tracing::dispatcher::get_default(Clone::clone);
        let restored = tokio::task::spawn_blocking(move || {
            tracing::dispatcher::with_default(&dispatcher, || {
                let result = git.restore(&GitRestore {
                    checkout: &request.checkout,
                    bundle: &bundle,
                    prior: &request.prior,
                    scratch: &scratch,
                    owner,
                    deadline,
                });
                // A canceled async caller cannot delete bytes still read by the blocking command.
                drop(directory);
                result
            })
        })
        .await;
        restored.unwrap_or(Err(RestoreFailure::Unavailable))
    }
}

/// Removes every restore directory a previous Node process left: no restore survives a restart.
pub(crate) fn purge(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// One restore's Node-private directory, removed when dropped.
struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    /// Creates the directory (and the root) owner-only, discarding what an earlier attempt left.
    fn create(path: &Path) -> io::Result<Self> {
        purge(path)?;
        fs::DirBuilder::new()
            .recursive(/*recursive*/ true)
            .mode(/*mode*/ 0o700)
            .create(path)?;
        Ok(Self(path.to_path_buf()))
    }
}

impl Drop for PrivateDirectory {
    /// A failure is only logged; the next Node start clears the root anyway.
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            ora_logging::ora_warn!(path = %self.0.display(), error = %error, "restore directory was not removed");
        }
    }
}

#[cfg(test)]
mod tests;

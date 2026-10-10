//! The Git steps of a restore (Node restore ADR D3), run with delivery's hardened policy.
//!
//! Every command goes through [`DeliveryGit`], so restore shares delivery's runner (host-managed,
//! as the workload user) and its fixed configuration (no hooks, no fsmonitor, no signing) on top
//! of clone's environment and protocol policy. The checkout's `origin` and its remote-tracking
//! branches are never changed; only the clone's branch, the index, the worktree and one ref under
//! `refs/ora/revisions/` are.
use super::super::snapshot::DeliveryGit;
use super::bundle::{BundleHeader, read_header};
use crate::session::{RestoreFailure, Restored};
use gitlancer::{GitExecError, GitRunner};
use ora_node_protocol::{CommitId, PriorRevision};
use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Background maintenance a fetch could start in the checkout; nothing may outlive the restore.
const QUIET_FETCH: [&str; 4] = ["-c", "gc.auto=0", "-c", "maintenance.auto=false"];

/// The verified bundle and where it goes.
pub(crate) struct GitRestore<'a> {
    pub(crate) checkout: &'a Path,
    /// The size- and digest-verified bundle in the Node-private directory.
    pub(crate) bundle: &'a Path,
    pub(crate) prior: &'a PriorRevision,
    /// Unique per execution, so leftover scratch files never collide.
    pub(crate) scratch: &'a str,
    /// The workload user restore Git runs as, when the deployment separates it from the Node.
    pub(crate) owner: Option<u32>,
}

impl<R: GitRunner> DeliveryGit<R> {
    /// Restores the prior Revision into the checkout and reports whether its base commits are
    /// still on the remote branch. Blocking: callers keep it off the async workers.
    pub(crate) fn restore(&self, request: &GitRestore<'_>) -> Result<Restored, RestoreFailure> {
        let checkout = request.checkout;
        let final_commit = &request.prior.final_commit;
        // 1. The header decides which failure a missing commit is, before Git reads the pack.
        let header = read_header(request.bundle).map_err(|error| {
            ora_logging::ora_warn!(error = %error, "prior bundle header was refused");
            RestoreFailure::Unavailable
        })?;
        if header.final_commit != *final_commit {
            ora_logging::ora_warn!("prior bundle does not hold the prior final commit");
            return Err(RestoreFailure::Unavailable);
        }
        let resolved = self
            .output(
                checkout,
                &[
                    "rev-parse",
                    "--absolute-git-dir",
                    "--symbolic-full-name",
                    "HEAD",
                ],
                &[],
            )
            .map_err(failed)?;
        let [git_dir, head] = resolved.lines().collect::<Vec<_>>()[..] else {
            return Err(RestoreFailure::Unavailable);
        };
        // The clone checks out a branch; anything else is not a checkout this Node produced.
        let Some(branch) = head.strip_prefix("refs/heads/") else {
            ora_logging::ora_warn!("restore checkout is not on a branch");
            return Err(RestoreFailure::Unavailable);
        };
        // 2. Base commits come only from the project repository, and only once.
        let mut missing = self.missing(checkout, &header.prerequisites)?;
        if !missing.is_empty() {
            let mut fetch = QUIET_FETCH.to_vec();
            fetch.extend(["fetch", "--no-tags", "origin"]);
            fetch.extend(missing.iter().map(CommitId::as_str));
            if let Err(error) = self.output(checkout, &fetch, &[]) {
                ora_logging::ora_warn!(error = %error, "base commits of the prior Revision could not be fetched");
                // Only the remote's own answer that it has no such object is permanent; a network
                // or access failure must not make Cloud refuse the Revision for every later run.
                if !remote_lacks_objects(&error) {
                    return Err(RestoreFailure::Unavailable);
                }
            }
            missing = self.missing(checkout, &missing)?;
            if !missing.is_empty() {
                return Err(RestoreFailure::BaseUnavailable);
            }
        }
        // 3-5. Git reads the bundle from the Git directory, which the workload user can read
        // while the Node-private copy is not reachable for it.
        let scratch = Scratch::copy(
            request.bundle,
            &Path::new(git_dir).join(format!("ora-restore-{}.bundle", request.scratch)),
            request.owner,
        )
        .map_err(|error| {
            ora_logging::ora_warn!(error = %error, "prior bundle could not be staged for Git");
            RestoreFailure::Unavailable
        })?;
        let bundle = scratch.0.to_str().ok_or(RestoreFailure::Unavailable)?;
        self.output(checkout, &["bundle", "verify", "-q", bundle], &[])
            .map_err(failed)?;
        let refspec = format!("{0}:{0}", header.head);
        let mut fetch = QUIET_FETCH.to_vec();
        // A local bundle is the file transport, which clone's protocol policy otherwise forbids.
        fetch.extend([
            "-c",
            "protocol.file.allow=always",
            "fetch",
            "--no-tags",
            bundle,
            &refspec,
        ]);
        self.output(checkout, &fetch, &[]).map_err(failed)?;
        drop(scratch);
        let peeled = format!("{}^{{commit}}", final_commit.as_str());
        let fetched = self
            .output(checkout, &["rev-parse", "--verify", "-q", &peeled], &[])
            .map_err(failed)?;
        if fetched != final_commit.as_str() {
            return Err(RestoreFailure::Unavailable);
        }
        self.output(
            checkout,
            &[
                "checkout",
                "-q",
                "--force",
                "-B",
                branch,
                final_commit.as_str(),
            ],
            &[],
        )
        .map_err(failed)?;
        // 6. A base the remote branch lost means its history was rewritten since.
        Ok(self.divergence(checkout, &header, branch))
    }

    /// The commits among `commits` the checkout does not have.
    fn missing(
        &self,
        checkout: &Path,
        commits: &[CommitId],
    ) -> Result<Vec<CommitId>, RestoreFailure> {
        let mut missing = Vec::new();
        for commit in commits {
            let object = format!("{}^{{commit}}", commit.as_str());
            match self.output(checkout, &["cat-file", "-e", &object], &[]) {
                Ok(_) => {}
                Err(GitExecError::NonZeroExit { .. }) => missing.push(commit.clone()),
                Err(error) => return Err(failed(error)),
            }
        }
        Ok(missing)
    }

    /// Names the first base commit `origin/<branch>` no longer contains. A runner failure here
    /// only loses the note, so it is logged and treated as no divergence.
    fn divergence(&self, checkout: &Path, header: &BundleHeader, branch: &str) -> Restored {
        let remote = format!("refs/remotes/origin/{branch}");
        for base in &header.prerequisites {
            match self.output(
                checkout,
                &["merge-base", "--is-ancestor", base.as_str(), &remote],
                &[],
            ) {
                Ok(_) => {}
                Err(GitExecError::NonZeroExit { .. }) => {
                    return Restored::Diverged {
                        final_commit: header.final_commit.clone(),
                        base_commit: base.clone(),
                        branch: branch.to_owned(),
                    };
                }
                Err(error) => {
                    ora_logging::ora_warn!(error = %error, "remote history of the restored Revision could not be checked");
                    return Restored::OnRemoteHistory;
                }
            }
        }
        Restored::OnRemoteHistory
    }
}

/// Keeps Git's diagnostics in the Node log while the session reports only the failure code.
fn failed(error: GitExecError) -> RestoreFailure {
    ora_logging::ora_warn!(error = %error, "prior Revision restore Git step failed");
    RestoreFailure::Unavailable
}

/// The bundle copy Git reads, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// Copies the verified bundle without following a link at the destination, owned by the
    /// workload user so its Git can read it.
    fn copy(source: &Path, destination: &Path, owner: Option<u32>) -> io::Result<Self> {
        match fs::remove_file(destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut output = fs::OpenOptions::new()
            .write(/*write*/ true)
            .create_new(/*create_new*/ true)
            .mode(/*mode*/ 0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(destination)?;
        let scratch = Self(destination.to_path_buf());
        io::copy(&mut fs::File::open(source)?, &mut output)?;
        output.sync_all()?;
        if let Some(owner) = owner {
            std::os::unix::fs::fchown(&output, Some(owner), Some(owner))?;
        }
        Ok(scratch)
    }
}

impl Drop for Scratch {
    /// The bundle is only needed while Git reads it.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Remote answers meaning the requested objects are not on the remote: the upload-pack refusals
/// for an unknown or unadvertised object ID. Anything else (unreachable host, refused access, a
/// broken connection) says nothing about the objects and stays a retryable failure.
const REMOTE_LACKS_OBJECTS: [&str; 4] = [
    "not our ref",
    "unadvertised object",
    "couldn't find remote ref",
    "no such remote ref",
];

/// Whether a failed base-commit fetch was the remote stating it does not have the objects.
fn remote_lacks_objects(error: &GitExecError) -> bool {
    match error {
        GitExecError::NonZeroExit { stderr, .. } => {
            let stderr = stderr.to_ascii_lowercase();
            REMOTE_LACKS_OBJECTS
                .iter()
                .any(|answer| stderr.contains(answer))
        }
        _ => false,
    }
}

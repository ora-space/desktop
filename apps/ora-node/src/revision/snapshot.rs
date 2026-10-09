//! The internal snapshot commit and the incremental bundle (Revision ADR D2, D3).
//!
//! Everything here runs Git plumbing against a scratch index, so the real index, the worktree
//! files and every branch stay as the Agent left them, and no commit hook can run. Scratch files
//! live inside the checkout's own Git directory: under a separate workload identity the Node home
//! is not writable by Git, while the Git directory always is.
use gitlancer::{GitCommand, GitEnv, GitExecError, GitIntent, GitRunner};
use ora_node_protocol::{CommitId, GitIdentity, RevisionRef};
use std::path::{Path, PathBuf};

/// Committer of every internal snapshot, distinct from the user it is authored for.
const COMMITTER_NAME: &str = "Ora";
const COMMITTER_EMAIL: &str = "revision@ora.invalid";
/// Fixed message so a snapshot is recognizably not a commit the Agent or user made.
const SNAPSHOT_MESSAGE: &str = "Ora snapshot of uncommitted run changes\n\nThis internal commit saves the files the run left uncommitted. It was created without\nrunning commit hooks and does not mean the project's checks passed.\n";
/// Settings no repository configuration may override for delivery: hooks and filesystem
/// monitors are programs the Agent could have configured in the checkout.
const DELIVERY_CONFIG: [&str; 3] = [
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "commit.gpgSign=false",
];
/// Captured output stays far below the guardian capture limit; plumbing prints object IDs only.
const OUTPUT_LIMIT: usize = 64 * 1024;

/// Hardened environment and per-command configuration shared with clone execution.
#[derive(Clone, Debug)]
pub(crate) struct GitPolicy {
    pub(crate) env: GitEnv,
    /// `-c key=value` pairs placed before every subcommand.
    pub(crate) config_args: Vec<String>,
}

/// Runs delivery Git with the deployment's policy, whatever runner executes it.
pub(crate) struct DeliveryGit<R> {
    runner: R,
    policy: GitPolicy,
}

/// What the snapshot produced; a bundle exists exactly when the final commit is not the base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Snapshot {
    Unchanged {
        final_commit: CommitId,
    },
    Changed {
        final_commit: CommitId,
        /// Verified bundle in the Git directory's scratch space, to be frozen by the caller.
        bundle: PathBuf,
    },
}

/// Which delivery step failed; raw Git diagnostics stay in Node logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotFailure {
    CheckoutUnavailable,
    SnapshotFailed,
    BundleFailed,
}

/// The delivery's checkout, target ref, base and author.
pub(crate) struct SnapshotRequest<'a> {
    pub(crate) checkout: &'a Path,
    pub(crate) revision_ref: &'a RevisionRef,
    pub(crate) base_commit: &'a CommitId,
    pub(crate) author: &'a GitIdentity,
    /// Unique per execution, so concurrent or leftover scratch files never collide.
    pub(crate) scratch: &'a str,
}

impl<R: GitRunner> DeliveryGit<R> {
    pub(crate) fn new(runner: R, policy: GitPolicy) -> Self {
        Self { runner, policy }
    }

    /// Saves the checkout's HEAD chain plus remaining changes under the Revision ref and bundles
    /// what the base commit lacks.
    pub(crate) fn snapshot(
        &self,
        request: &SnapshotRequest<'_>,
    ) -> Result<Snapshot, SnapshotFailure> {
        let checkout = request.checkout;
        // One Run resolves everything the snapshot starts from: each host-managed Git Run has a
        // fixed setup cost. Revisions before `--` must all exist (Git echoes the `--`), so a
        // checkout without the base commit fails here.
        let base = format!("{}^{{commit}}", request.base_commit.as_str());
        let resolved = self
            .output(
                checkout,
                &[
                    "rev-parse",
                    "--absolute-git-dir",
                    "HEAD^{commit}",
                    "HEAD^{tree}",
                    &base,
                    "--",
                ],
                &[],
            )
            .map_err(|error| log(error, SnapshotFailure::CheckoutUnavailable))?;
        let [git_dir, head, head_tree, _base, "--"] = resolved.lines().collect::<Vec<_>>()[..]
        else {
            ora_logging::ora_warn!("Revision delivery could not resolve its checkout");
            return Err(SnapshotFailure::CheckoutUnavailable);
        };
        let (git_dir, head, head_tree) = (PathBuf::from(git_dir), head.to_owned(), head_tree);
        let index = git_dir.join(format!("ora-revision-{}.index", request.scratch));
        let bundle = git_dir.join(format!("ora-revision-{}.bundle", request.scratch));
        // Leftovers of an attempt interrupted by a restart are discarded: nothing was frozen yet.
        for stale in [&index, &index.with_extension("index.lock"), &bundle] {
            let _ = std::fs::remove_file(stale);
        }
        let tree = self.scratch_tree(checkout, &index);
        let _ = std::fs::remove_file(&index);
        let tree = tree.map_err(|error| log(error, SnapshotFailure::SnapshotFailed))?;
        // Comparing trees rather than parsing `git status` keeps the change probe bounded: it
        // answers exactly "would the scratch commit differ from HEAD" for tracked changes,
        // deletions and non-ignored new files, however many files changed.
        let final_commit = if tree == head_tree {
            head
        } else {
            let author = [
                ("GIT_AUTHOR_NAME", request.author.name.as_str()),
                ("GIT_AUTHOR_EMAIL", request.author.email.as_str()),
                ("GIT_COMMITTER_NAME", COMMITTER_NAME),
                ("GIT_COMMITTER_EMAIL", COMMITTER_EMAIL),
            ];
            self.output(
                checkout,
                &[
                    "commit-tree",
                    "--no-gpg-sign",
                    &tree,
                    "-p",
                    &head,
                    "-m",
                    SNAPSHOT_MESSAGE,
                ],
                &author,
            )
            .map_err(|error| log(error, SnapshotFailure::SnapshotFailed))?
        };
        self.output(
            checkout,
            &["update-ref", request.revision_ref.as_str(), &final_commit],
            &[],
        )
        .map_err(|error| log(error, SnapshotFailure::SnapshotFailed))?;
        let final_commit = CommitId::new(final_commit);
        if final_commit == *request.base_commit {
            return Ok(Snapshot::Unchanged { final_commit });
        }
        let path = bundle
            .to_str()
            .ok_or(SnapshotFailure::BundleFailed)?
            .to_owned();
        let excluded = format!("^{}", request.base_commit.as_str());
        let created = self
            .output(
                checkout,
                &[
                    "bundle",
                    "create",
                    "-q",
                    &path,
                    request.revision_ref.as_str(),
                    &excluded,
                ],
                &[],
            )
            .and_then(|_| self.output(checkout, &["bundle", "verify", "-q", &path], &[]));
        if let Err(error) = created {
            let _ = std::fs::remove_file(&bundle);
            return Err(log(error, SnapshotFailure::BundleFailed));
        }
        Ok(Snapshot::Changed {
            final_commit,
            bundle,
        })
    }

    /// Stages HEAD plus the worktree into the scratch index and writes its tree.
    fn scratch_tree(&self, checkout: &Path, index: &Path) -> Result<String, GitExecError> {
        let index = index
            .to_str()
            .ok_or_else(|| GitExecError::OutputReadFailed {
                stream: "scratch index path",
                source: std::io::Error::other("non-UTF-8 Git directory"),
            })?;
        let scratch = [("GIT_INDEX_FILE", index)];
        self.output(checkout, &["read-tree", "HEAD"], &scratch)?;
        self.output(checkout, &["add", "-A"], &scratch)?;
        self.output(checkout, &["write-tree"], &scratch)
    }

    /// Runs one hardened command in the checkout and returns its trimmed standard output.
    fn output(
        &self,
        checkout: &Path,
        args: &[&str],
        variables: &[(&str, &str)],
    ) -> Result<String, GitExecError> {
        let mut env = self.policy.env.clone();
        for (name, value) in variables {
            env = env.with_variable(*name, *value);
        }
        let mut full = self.policy.config_args.clone();
        for config in DELIVERY_CONFIG {
            full.extend(["-c".to_owned(), config.to_owned()]);
        }
        full.extend(args.iter().map(|arg| (*arg).to_owned()));
        let command = GitCommand::new(checkout.to_path_buf(), full, env, GitIntent::Mutating);
        let output = self
            .runner
            .run_bounded(&command, OUTPUT_LIMIT, OUTPUT_LIMIT)?;
        if output.code != Some(0) {
            return Err(GitExecError::NonZeroExit {
                code: output.code,
                args: command.args,
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        Ok(output.stdout.trim().to_owned())
    }
}

/// Keeps Git's diagnostics in the Node log while the protocol carries only the failure code.
fn log(error: GitExecError, failure: SnapshotFailure) -> SnapshotFailure {
    ora_logging::ora_warn!(failure = ?failure, error = %error, "Revision delivery Git step failed");
    failure
}

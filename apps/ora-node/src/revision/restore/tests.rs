#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Restores against real repositories: a prior run's checkout, the bundle its delivery created,
//! and the fresh clone of a new run, with `origin` as a local `file://` repository.
mod git;
mod grants;
mod restorer;

use super::super::snapshot::{DeliveryGit, GitPolicy, Snapshot, SnapshotRequest};
use gitlancer::{CliGitRunner, GitEnv};
use ora_node_protocol::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const REVISION_REF: &str = "refs/ora/revisions/run-1";

/// Runs fixture Git with an isolated configuration; this is setup, not the code under test.
fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Delivery's Git with a plain runner and an isolated configuration, as production composes it
/// with the host runner and clone's policy.
fn delivery_git() -> DeliveryGit<CliGitRunner> {
    DeliveryGit::new(
        CliGitRunner,
        GitPolicy {
            env: GitEnv::automation_defaults()
                .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
            config_args: vec![],
        },
    )
}

/// An origin with one base commit, a prior run that committed work on it, and the bundle that
/// run's delivery produced.
struct World {
    directory: tempfile::TempDir,
    base: CommitId,
    prior: PriorRevision,
}

impl World {
    fn new() -> Self {
        ora_logging::initialize_test_clock();
        let directory = tempfile::tempdir().unwrap();
        let origin = directory.path().join("origin");
        fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-q"]);
        fs::write(origin.join("README.md"), "base\n").unwrap();
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-q", "-m", "base"]);
        let base = CommitId::new(git(&origin, &["rev-parse", "HEAD"]));
        let world = Self {
            directory,
            base: base.clone(),
            prior: PriorRevision {
                revision_id: RevisionId::new("revision-1"),
                final_commit: base,
                bundle: StoredObject {
                    key: ObjectKey::new("runs/1/revision.bundle"),
                    size: 0,
                    sha256: Sha256Digest::new(""),
                },
            },
        };
        world.clone_into("prior");
        let prior = world.path("prior");
        fs::write(prior.join("work.txt"), "committed work\n").unwrap();
        git(&prior, &["add", "-A"]);
        git(&prior, &["commit", "-q", "-m", "work"]);
        // Left uncommitted, so the delivery's own snapshot commit is part of the prior Revision.
        fs::write(prior.join("notes.txt"), "uncommitted work\n").unwrap();
        let Snapshot::Changed {
            final_commit,
            bundle,
        } = delivery_git()
            .snapshot(&SnapshotRequest {
                checkout: &prior,
                revision_ref: &RevisionRef::new(REVISION_REF),
                base_commit: &world.base,
                prior_final_commit: None,
                author: &GitIdentity {
                    name: "Session User".into(),
                    email: "user@example.com".into(),
                },
                scratch: "prior",
            })
            .unwrap()
        else {
            panic!("the prior run changed the checkout");
        };
        fs::rename(&bundle, world.bundle()).unwrap();
        let bytes = fs::read(world.bundle()).unwrap();
        Self {
            prior: PriorRevision {
                final_commit,
                bundle: StoredObject {
                    key: world.prior.bundle.key.clone(),
                    size: bytes.len() as u64,
                    sha256: Sha256Digest::new(ora_utils::hash::sha256_hex(&bytes)),
                },
                ..world.prior.clone()
            },
            ..world
        }
    }

    /// A path inside the fixture directory.
    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    /// The project repository every checkout clones.
    fn origin(&self) -> PathBuf {
        self.path("origin")
    }

    /// The prior delivery's verified bundle, as the restore downloads it.
    fn bundle(&self) -> PathBuf {
        self.path("prior.bundle")
    }

    /// Clones `origin` over its transport, as the Node's clone does, so nothing is shared.
    fn clone_into(&self, name: &str) -> PathBuf {
        let url = format!("file://{}", self.origin().display());
        git(
            self.directory.path(),
            &["clone", "-q", "--single-branch", &url, name],
        );
        self.path(name)
    }

    /// Rewrites `origin/main` onto unrelated history, keeping the base reachable from `keep` only
    /// when asked; without it, the base is collected and no longer fetchable.
    fn rewrite_origin(&self, keep_base: bool) {
        let origin = self.origin();
        if keep_base {
            git(&origin, &["branch", "keep"]);
        }
        git(&origin, &["checkout", "-q", "--orphan", "rewritten"]);
        fs::write(origin.join("README.md"), "rewritten\n").unwrap();
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-q", "-m", "rewritten"]);
        git(&origin, &["branch", "-f", "main", "rewritten"]);
        git(&origin, &["checkout", "-q", "main"]);
        git(&origin, &["branch", "-q", "-D", "rewritten"]);
        git(&origin, &["reflog", "expire", "--expire=now", "--all"]);
        git(&origin, &["gc", "-q", "--prune=now"]);
    }
}

/// What a restore may change, read independently of the code under test.
#[derive(Debug, PartialEq, Eq)]
struct CheckoutState {
    head: String,
    commit: String,
    status: String,
    revision_ref: String,
    origin_main: String,
    restore_scratch: Vec<String>,
}

/// Reads the checkout's branch, commit, worktree status, Revision ref and remote branch.
fn state(checkout: &Path) -> CheckoutState {
    let git_dir = checkout.join(".git");
    let mut restore_scratch: Vec<String> = fs::read_dir(&git_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ora-restore-"))
        .collect();
    restore_scratch.sort();
    CheckoutState {
        head: git(checkout, &["symbolic-ref", "HEAD"]),
        commit: git(checkout, &["rev-parse", "HEAD"]),
        status: git(checkout, &["status", "--porcelain"]),
        revision_ref: Command::new("git")
            .current_dir(checkout)
            .args(["rev-parse", "-q", "--verify", REVISION_REF])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap(),
        origin_main: git(checkout, &["rev-parse", "refs/remotes/origin/main"]),
        restore_scratch,
    }
}

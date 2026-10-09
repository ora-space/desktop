//! Preparation against real repositories: what the snapshot saves, what it leaves untouched, and
//! that the bundle restores the final commit on top of the base alone.
use super::super::prepare::directory_name;
use super::*;
use gitlancer::{CliGitRunner, GitEnv};
use ora_node_db::{DeliveryPlan, FrozenOutcome};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// An origin with one base commit, the checkout cloned from it, and a sealed history.
struct Fixture {
    directory: tempfile::TempDir,
    base: CommitId,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let origin = directory.path().join("origin");
        fs::create_dir(&origin).unwrap();
        git(&origin, &["init"]);
        fs::write(origin.join("kept.txt"), "kept\n").unwrap();
        fs::write(origin.join("edited.txt"), "before\n").unwrap();
        fs::write(origin.join("deleted.txt"), "deleted\n").unwrap();
        fs::write(origin.join(".gitignore"), "*.log\n").unwrap();
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-m", "base"]);
        let base = CommitId::new(git(&origin, &["rev-parse", "HEAD"]));
        git(directory.path(), &["clone", "-q", "origin", "checkout"]);
        fs::write(
            directory.path().join("history.jsonl"),
            "{\"type\":\"turnEnded\"}\n",
        )
        .unwrap();
        Self { directory, base }
    }

    fn checkout(&self) -> PathBuf {
        self.directory.path().join("checkout")
    }

    fn root(&self) -> PathBuf {
        self.directory.path().join("frozen")
    }

    /// The delivery of this fixture's checkout and history.
    fn job(&self) -> Preparation {
        Preparation {
            execution: ExecutionId::new("deliver-execution"),
            spec: DeliverRevisionSpec {
                node_id: NodeId::new("node"),
                session_execution_id: ExecutionId::new("session"),
                checkout_execution_id: ExecutionId::new("clone"),
                base_commit: self.base.clone(),
                revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
                bundle_key: ObjectKey::new("runs/1/revision.bundle"),
                history_key: ObjectKey::new("runs/1/history.jsonl"),
            },
            checkout: self.checkout(),
            history: self.directory.path().join("history.jsonl"),
            author: GitIdentity {
                name: "Session User".into(),
                email: "user@example.com".into(),
            },
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("first"),
            },
        }
    }

    /// Prepares through the production code path with a plain Git runner.
    fn prepare(&self, job: &Preparation) -> Result<DeliveryPlan, RevisionFailureCode> {
        let git = DeliveryGit::new(
            CliGitRunner,
            GitPolicy {
                env: GitEnv::automation_defaults()
                    .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                    .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                config_args: vec![],
            },
        );
        prepare(&git, &self.root(), job)
    }

    /// Every worktree file outside `.git` with its content, plus the real index bytes, the
    /// current branch and its tip: everything the snapshot must not change.
    fn untouched_state(&self) -> (BTreeMap<String, Vec<u8>>, Vec<u8>, String, String) {
        let checkout = self.checkout();
        let mut files = BTreeMap::new();
        let mut pending = vec![checkout.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().unwrap() == ".git" {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let name = path
                        .strip_prefix(&checkout)
                        .unwrap()
                        .to_string_lossy()
                        .into();
                    files.insert(name, fs::read(&path).unwrap());
                }
            }
        }
        (
            files,
            fs::read(checkout.join(".git").join("index")).unwrap(),
            git(&checkout, &["symbolic-ref", "HEAD"]),
            git(&checkout, &["rev-parse", "HEAD"]),
        )
    }
}

/// Measures a frozen file independently of the production helper.
fn measured(key: &str, path: &Path) -> StoredObject {
    let bytes = fs::read(path).unwrap();
    StoredObject {
        key: ObjectKey::new(key),
        size: bytes.len() as u64,
        sha256: Sha256Digest::new(ora_utils::hash::sha256_hex(&bytes)),
    }
}

/// With nothing to save the result is `unchanged`, the ref names the base and no bundle exists.
#[test]
fn unchanged_checkout_freezes_only_the_history() {
    let fixture = Fixture::new();
    let job = fixture.job();
    // An ignored file is not a change.
    fs::write(fixture.checkout().join("debug.log"), "noise").unwrap();
    let plan = fixture.prepare(&job).unwrap();
    let frozen = fixture.root().join(directory_name(&job.execution));
    assert_eq!(
        plan,
        DeliveryPlan {
            directory: directory_name(&job.execution),
            outcome: FrozenOutcome::Unchanged(RevisionUnchanged {
                node: job.node.clone(),
                final_commit: fixture.base.clone(),
                base_commit: fixture.base.clone(),
                revision_ref: job.spec.revision_ref.clone(),
                history: measured("runs/1/history.jsonl", &frozen.join("history.jsonl")),
            }),
        }
    );
    assert_eq!(
        fs::read(frozen.join("history.jsonl")).unwrap(),
        fs::read(&job.history).unwrap()
    );
    assert!(!frozen.join("revision.bundle").exists());
    assert_eq!(
        git(
            &fixture.checkout(),
            &["rev-parse", "refs/ora/revisions/run-1"]
        ),
        fixture.base.as_str()
    );
}

/// Agent commits are kept, remaining edits, deletions and new files become one hookless snapshot
/// commit, ignored files stay out, and the user's index, files and branch are untouched; the
/// bundle then restores the final commit in a repository that has only the base.
#[test]
fn changes_become_a_hookless_snapshot_bundled_against_the_base() {
    let fixture = Fixture::new();
    let checkout = fixture.checkout();
    fs::write(checkout.join("agent.txt"), "committed by the agent\n").unwrap();
    git(&checkout, &["add", "agent.txt"]);
    git(&checkout, &["commit", "-m", "agent work"]);
    let agent_commit = git(&checkout, &["rev-parse", "HEAD"]);
    for hook in ["pre-commit", "commit-msg", "post-commit"] {
        let path = checkout.join(".git").join("hooks").join(hook);
        fs::write(
            &path,
            "#!/bin/sh\ntouch \"$(dirname \"$0\")/../../hook-ran\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(/*mode*/ 0o755)).unwrap();
    }
    fs::write(checkout.join("edited.txt"), "after\n").unwrap();
    fs::remove_file(checkout.join("deleted.txt")).unwrap();
    fs::write(checkout.join("added.txt"), "new\n").unwrap();
    fs::write(checkout.join("build.log"), "ignored\n").unwrap();
    // A partially staged change must stay staged exactly as it was.
    git(&checkout, &["add", "edited.txt"]);
    fs::write(checkout.join("edited.txt"), "after, then more\n").unwrap();
    let before = fixture.untouched_state();
    let job = fixture.job();
    let plan = fixture.prepare(&job).unwrap();
    assert_eq!(fixture.untouched_state(), before);
    assert!(!checkout.join("hook-ran").exists());
    let FrozenOutcome::Delivered(delivered) = &plan.outcome else {
        panic!("expected a delivered plan, got {plan:?}");
    };
    let final_commit = delivered.final_commit.as_str();
    assert_eq!(
        git(&checkout, &["rev-parse", "refs/ora/revisions/run-1"]),
        final_commit
    );
    assert_eq!(
        git(
            &checkout,
            &["log", "-1", "--format=%P|%an <%ae>|%cn <%ce>", final_commit]
        ),
        format!("{agent_commit}|Session User <user@example.com>|Ora <revision@ora.invalid>")
    );
    assert_eq!(
        git(&checkout, &["ls-tree", "-r", "--name-only", final_commit]),
        ".gitignore\nadded.txt\nagent.txt\nedited.txt\nkept.txt"
    );
    assert_eq!(
        git(&checkout, &["show", &format!("{final_commit}:edited.txt")]),
        "after, then more"
    );
    let frozen = fixture.root().join(&plan.directory);
    assert_eq!(
        plan.outcome,
        FrozenOutcome::Delivered(RevisionDelivered {
            node: job.node.clone(),
            final_commit: delivered.final_commit.clone(),
            base_commit: fixture.base.clone(),
            revision_ref: job.spec.revision_ref,
            bundle: measured("runs/1/revision.bundle", &frozen.join("revision.bundle")),
            history: measured("runs/1/history.jsonl", &frozen.join("history.jsonl")),
        })
    );
    // Scratch files in the Git directory are gone once the bundle is frozen.
    let scratch: Vec<_> = fs::read_dir(checkout.join(".git"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ora-revision-"))
        .collect();
    assert_eq!(scratch, Vec::<String>::new());
    let restored = fixture.directory.path().join("restored");
    git(
        fixture.directory.path(),
        &["clone", "-q", "--no-local", "origin", "restored"],
    );
    let bundle = frozen.join("revision.bundle");
    git(
        &restored,
        &[
            "fetch",
            "-q",
            bundle.to_str().unwrap(),
            "refs/ora/revisions/run-1:refs/heads/restored",
        ],
    );
    assert_eq!(git(&restored, &["rev-parse", "restored"]), final_commit);
}

/// A checkout without the base, a directory that is no repository and a missing history fail
/// with their own codes and leave nothing frozen.
#[test]
fn unusable_inputs_fail_without_frozen_objects() {
    let fixture = Fixture::new();
    let mut missing_base = fixture.job();
    missing_base.spec.base_commit = CommitId::new("1".repeat(40));
    let mut not_a_repository = fixture.job();
    not_a_repository.checkout = fixture.directory.path().to_path_buf();
    let mut missing_history = fixture.job();
    missing_history.history = fixture.directory.path().join("absent.jsonl");
    for (job, failure) in [
        (missing_base, RevisionFailureCode::CheckoutUnavailable),
        (not_a_repository, RevisionFailureCode::CheckoutUnavailable),
        (missing_history, RevisionFailureCode::HistoryUnavailable),
    ] {
        assert_eq!(fixture.prepare(&job), Err(failure));
        assert!(!fixture.root().join(directory_name(&job.execution)).exists());
    }
}

/// Re-preparing an execution that never froze replaces whatever an interrupted attempt left.
#[test]
fn preparing_again_discards_an_abandoned_attempt() {
    let fixture = Fixture::new();
    let job = fixture.job();
    let leftover = fixture.root().join(directory_name(&job.execution));
    fs::create_dir_all(&leftover).unwrap();
    fs::write(leftover.join("revision.bundle"), "partial").unwrap();
    let plan = fixture.prepare(&job).unwrap();
    assert!(matches!(plan.outcome, FrozenOutcome::Unchanged(_)));
    assert!(!leftover.join("revision.bundle").exists());
}

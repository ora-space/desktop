//! Preparation against real repositories: what the snapshot saves, what it leaves untouched, and
//! and which final commit it freezes.
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

/// Whole-run delivery keeps failed and successful attempts byte-for-byte in one Git snapshot.
#[test]
fn workflow_snapshot_freezes_every_history_and_explicit_unstarted_attempt() {
    let fixture = Fixture::new();
    let mut job = fixture.job();
    job.spec.session_execution_id = ExecutionId::new("");
    let attempts: Vec<_> = (0..3)
        .map(|index| WorkflowSessionAttempt {
            execution_order: index,
            node_id: if index < 2 { "a" } else { "b" }.into(),
            attempt: if index == 1 { 2 } else { 1 },
            execution_id: ExecutionId::new(format!("session-{index}")),
            history_key: ObjectKey::new(format!("run/{index}.jsonl")),
        })
        .collect();
    job.spec.workflow = Some(WorkflowHistoryInput {
        workflow_run_id: "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a".into(),
        sessions: attempts.clone(),
    });
    let raw = [
        b"{\"failed\": true}\r\n".as_slice(),
        b"{\"success\": 1e3}\n".as_slice(),
    ];
    let mut sources = Vec::new();
    for (index, attempt) in attempts.iter().enumerate() {
        let path = fixture.directory.path().join(format!("raw-{index}"));
        if let Some(bytes) = raw.get(index) {
            fs::write(&path, bytes).unwrap();
            sources.push((attempt.clone(), Some(path)));
        } else {
            sources.push((attempt.clone(), None));
        }
    }
    fs::write(fixture.checkout().join("whole-run.txt"), "both nodes' code").unwrap();
    job.history = prepare::HistoryInput::Workflow(sources.clone());
    let plan = fixture.prepare(&job).unwrap();
    let frozen = fixture.root().join(&plan.directory);
    assert_eq!(plan.histories.len(), 2);
    for (index, object) in plan.histories.iter().enumerate() {
        assert_eq!(
            fs::read(frozen.join(crate::revision::workflow::object_file(&object.key))).unwrap(),
            raw[index]
        );
    }
    let manifest: WorkflowHistoryManifest =
        serde_json::from_slice(&fs::read(frozen.join(HISTORY_FILE)).unwrap()).unwrap();
    assert_eq!(manifest.sessions.len(), 3);
    assert_eq!(
        manifest.sessions[2].history,
        WorkflowSessionHistory::NotStarted
    );
    let final_commit = match plan.outcome {
        FrozenOutcome::Delivered(ref r) => r.final_commit.as_str(),
        _ => panic!("code change missing"),
    };
    assert_eq!(
        git(
            &fixture.checkout(),
            &["show", &format!("{final_commit}:whole-run.txt")]
        ),
        "both nodes' code"
    );
    // Established history loss cannot become a smaller successful manifest.
    fs::remove_file(sources[0].1.as_ref().unwrap()).unwrap();
    assert_eq!(
        fixture.prepare(&job),
        Err(RevisionFailureCode::HistoryUnavailable)
    );
}

/// Rejects every operation except the one HEAD read allowed to an incomplete, unstarted restore.
struct HeadOnly(std::sync::atomic::AtomicUsize);

impl gitlancer::GitRunner for HeadOnly {
    /// A scratch-index probe, update-ref or bundle command makes this regression fail directly.
    fn run(
        &self,
        command: &gitlancer::GitCommand,
    ) -> Result<gitlancer::GitOutput, gitlancer::GitExecError> {
        self.run_bounded(command, usize::MAX, usize::MAX)
    }

    /// Delegates the permitted read to real Git without modifying process environment.
    fn run_bounded(
        &self,
        command: &gitlancer::GitCommand,
        stdout: usize,
        stderr: usize,
    ) -> Result<gitlancer::GitOutput, gitlancer::GitExecError> {
        assert!(command.args.ends_with(&[
            "rev-parse".to_string(),
            "--verify".to_string(),
            "HEAD^{commit}".to_string()
        ]));
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        CliGitRunner.run_bounded(command, stdout, stderr)
    }
}

/// Partial checkout files are never turned into new content at either permitted HEAD.
#[test]
fn unstarted_partial_restore_delivers_only_head_without_probing_files() {
    let dispatcher = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    );
    tracing::dispatcher::with_default(&dispatcher, || {
        for restored_head in [false, true] {
            let fixture = Fixture::new();
            let checkout = fixture.checkout();
            let mut job = fixture.job();
            job.mode = SnapshotMode::UnstartedRestore;
            fs::write(checkout.join("edited.txt"), "prior committed content\n").unwrap();
            git(&checkout, &["add", "-A"]);
            git(&checkout, &["commit", "-m", "prior"]);
            let prior = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
            job.spec.prior_revision = Some(PriorRevisionCommit {
                revision_id: RevisionId::new("prior"),
                final_commit: prior.clone(),
            });
            if !restored_head {
                git(&checkout, &["reset", "--hard", fixture.base.as_str()]);
            }
            // A failed/killed checkout may leave both tracked modifications and new partial files.
            fs::write(checkout.join("edited.txt"), "partial checkout content\n").unwrap();
            fs::write(checkout.join("partial.txt"), "must never be delivered\n").unwrap();
            fs::write(
                checkout.join(".git/index.lock"),
                "retained failure evidence\n",
            )
            .unwrap();
            let before = fixture.untouched_state();
            let runner = HeadOnly(std::sync::atomic::AtomicUsize::new(/*v*/ 0));
            let git = DeliveryGit::new(
                runner,
                GitPolicy {
                    env: GitEnv::automation_defaults()
                        .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                        .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                    config_args: vec![],
                },
            );
            let plan = prepare(&git, &fixture.root(), &job).unwrap();
            let FrozenOutcome::Unchanged(result) = plan.outcome else {
                panic!("partial restore was delivered");
            };
            assert_eq!(
                result.final_commit,
                if restored_head {
                    prior
                } else {
                    fixture.base.clone()
                }
            );
            assert_eq!(git.runner.0.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(fixture.untouched_state(), before);
            assert!(fs::read_dir(checkout.join(".git/refs/ora/revisions")).is_err());
        }
    });
}

/// An origin with one base commit, the checkout cloned from it, and a sealed history.
struct Fixture {
    directory: tempfile::TempDir,
    base: CommitId,
}

/// A real non-atomic checkout failure leaves early files at the target while HEAD is unchanged.
#[test]
fn failed_checkout_filter_cannot_turn_partial_files_into_a_revision() {
    let subscriber = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    );
    tracing::dispatcher::with_default(&subscriber, || {
        let fixture = Fixture::new();
        let checkout = fixture.checkout();
        fs::write(
            checkout.join(".gitattributes"),
            "kept.txt filter=ora-broken\n",
        )
        .unwrap();
        fs::write(checkout.join("edited.txt"), "restored first\n").unwrap();
        fs::write(checkout.join("kept.txt"), "restored last\n").unwrap();
        git(&checkout, &["add", "-A"]);
        git(&checkout, &["commit", "-m", "prior target"]);
        let prior = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
        git(&checkout, &["reset", "--hard", fixture.base.as_str()]);
        git(&checkout, &["config", "filter.ora-broken.smudge", "exit 1"]);
        git(&checkout, &["config", "filter.ora-broken.required", "true"]);
        let failed = std::process::Command::new("git")
            .current_dir(&checkout)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["checkout", "--force", "-B", "main", prior.as_str()])
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("smudge filter"));
        assert_eq!(
            fs::read_to_string(checkout.join("edited.txt")).unwrap(),
            "restored first\n"
        );
        let head = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
        assert_eq!(head, fixture.base);
        let before = fixture.untouched_state();
        let mut job = fixture.job();
        job.mode = SnapshotMode::UnstartedRestore;
        job.spec.prior_revision = Some(PriorRevisionCommit {
            revision_id: RevisionId::new("prior"),
            final_commit: prior,
        });
        let runner = HeadOnly(std::sync::atomic::AtomicUsize::new(/*v*/ 0));
        let git = DeliveryGit::new(
            runner,
            GitPolicy {
                env: GitEnv::automation_defaults()
                    .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                    .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                config_args: vec![],
            },
        );
        let plan = prepare(&git, &fixture.root(), &job).unwrap();
        let FrozenOutcome::Unchanged(result) = plan.outcome else {
            panic!("failed checkout became a deliverable snapshot");
        };
        assert_eq!(result.final_commit, head);
        assert_eq!(git.runner.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(fixture.untouched_state(), before);
    });
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
            mode: crate::revision::SnapshotMode::Worktree,
            execution: ExecutionId::new("deliver-execution"),
            spec: DeliverRevisionSpec {
                workflow: None,
                node_id: NodeId::new("node"),
                session_execution_id: ExecutionId::new("session"),
                checkout_execution_id: ExecutionId::new("clone"),
                base_commit: self.base.clone(),
                revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
                history_key: ObjectKey::new("runs/1/history.jsonl"),
                prior_revision: None,
                git_repository_url: GitRepositoryUrl::parse("https://git.ora.test/t/p.git")
                    .unwrap(),
                delivery_ref: DeliveryRef::parse("refs/ora/deliveries/work-1").unwrap(),
                workspace_ref: WorkspaceRef::parse(
                    "refs/heads/workspaces/4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a/main",
                )
                .unwrap(),
            },
            checkout: self.checkout(),
            history: prepare::HistoryInput::Session(self.directory.path().join("history.jsonl")),
            author: GitIdentity {
                name: "Session User".into(),
                email: "user@example.com".into(),
            },
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("first"),
            },
            // Unprivileged tests can only hand the tree to themselves; that still runs the
            // no-follow ownership walk on every preparation, as production does.
            owner: Some(std::os::unix::fs::MetadataExt::uid(
                &std::fs::metadata(self.checkout()).expect("checkout metadata"),
            )),
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

/// With nothing to save the result is `unchanged`, the ref names the base, and only the history
/// is frozen.
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
            histories: Vec::new(),
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
        fs::read(fixture.directory.path().join("history.jsonl")).unwrap()
    );
    assert_eq!(
        fs::read_dir(&frozen).unwrap().count(),
        1,
        "only the history is frozen"
    );
    assert_eq!(
        git(
            &fixture.checkout(),
            &["rev-parse", "refs/ora/revisions/run-1"]
        ),
        fixture.base.as_str()
    );
}

/// A resumed session that added nothing stops at the prior Revision's final commit: the delivery
/// is unchanged at that commit, since the prior Revision already holds it (restore ADR D5). Any
/// further change is delivered as usual.
#[test]
fn resumed_delivery_stopping_at_the_prior_final_commit_is_unchanged() {
    let fixture = Fixture::new();
    let checkout = fixture.checkout();
    fs::write(checkout.join("restored.txt"), "prior run\n").unwrap();
    git(&checkout, &["add", "-A"]);
    git(&checkout, &["commit", "-m", "prior run"]);
    let prior_final = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
    let mut job = fixture.job();
    job.spec.prior_revision = Some(PriorRevisionCommit {
        revision_id: RevisionId::new("revision-1"),
        final_commit: prior_final.clone(),
    });
    let plan = fixture.prepare(&job).unwrap();
    let frozen = fixture.root().join(directory_name(&job.execution));
    assert_eq!(
        plan,
        DeliveryPlan {
            histories: Vec::new(),
            directory: directory_name(&job.execution),
            outcome: FrozenOutcome::Unchanged(RevisionUnchanged {
                node: job.node.clone(),
                final_commit: prior_final.clone(),
                base_commit: fixture.base.clone(),
                revision_ref: job.spec.revision_ref.clone(),
                history: measured("runs/1/history.jsonl", &frozen.join("history.jsonl")),
            }),
        }
    );

    fs::write(checkout.join("more.txt"), "this run\n").unwrap();
    let FrozenOutcome::Delivered(delivered) = fixture.prepare(&job).unwrap().outcome else {
        panic!("new work after the prior final commit is delivered");
    };
    assert_eq!(
        (
            delivered.base_commit,
            git(
                &checkout,
                &[
                    "rev-parse",
                    &format!("{}^", delivered.final_commit.as_str())
                ]
            )
        ),
        (fixture.base.clone(), prior_final.as_str().to_owned())
    );
}

/// Agent commits are kept, remaining edits, deletions and new files become one hookless snapshot
/// commit, ignored files stay out, and the user's index, files and branch are untouched.
#[test]
fn changes_become_a_hookless_snapshot_on_top_of_the_agent_commits() {
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
            history: measured("runs/1/history.jsonl", &frozen.join("history.jsonl")),
        })
    );
    // Scratch files in the Git directory are gone once the plan is frozen.
    let scratch: Vec<_> = fs::read_dir(checkout.join(".git"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ora-revision-"))
        .collect();
    assert_eq!(scratch, Vec::<String>::new());
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
    missing_history.history =
        prepare::HistoryInput::Session(fixture.directory.path().join("absent.jsonl"));
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
    fs::write(leftover.join("partial"), "partial").unwrap();
    let plan = fixture.prepare(&job).unwrap();
    assert!(matches!(plan.outcome, FrozenOutcome::Unchanged(_)));
    assert!(!leftover.join("partial").exists());
}

/// The reserved artifact directory never enters a snapshot's tree, so an artifact can never be
/// published in a delivery ref: those files are registered as objects with their own visibility,
/// retention and signed downloads. The exclusion applies to every run and leaves the worktree and
/// the user's index exactly as the Agent left them; a copy the Agent already committed stays in
/// history but stops being updated by later snapshots.
#[test]
fn the_reserved_artifact_directory_never_enters_the_snapshot() {
    let fixture = Fixture::new();
    let checkout = fixture.checkout();
    let artifacts = checkout.join(SCHEDULED_ARTIFACT_DIRECTORY);
    fs::create_dir_all(artifacts.join("nested")).unwrap();
    fs::write(artifacts.join("kept.md"), "committed earlier\n").unwrap();
    git(&checkout, &["add", ".ora-scheduled-artifacts/kept.md"]);
    git(&checkout, &["commit", "-m", "agent commits an artifact"]);
    fs::write(artifacts.join("kept.md"), "rewritten after the commit\n").unwrap();
    fs::write(artifacts.join("report.md"), "private report\n").unwrap();
    fs::write(artifacts.join("nested/coverage.xml"), "<coverage/>\n").unwrap();
    fs::write(checkout.join("edited.txt"), "delivered content\n").unwrap();
    let before = fixture.untouched_state();
    let plan = fixture.prepare(&fixture.job()).unwrap();
    assert_eq!(fixture.untouched_state(), before);

    let FrozenOutcome::Delivered(delivered) = &plan.outcome else {
        panic!("expected a delivered plan, got {plan:?}");
    };
    let final_commit = delivered.final_commit.as_str();
    assert_eq!(
        git(&checkout, &["ls-tree", "-r", "--name-only", final_commit])
            .lines()
            .collect::<Vec<_>>(),
        vec![
            ".gitignore",
            ".ora-scheduled-artifacts/kept.md",
            "deleted.txt",
            "edited.txt",
            "kept.txt",
        ]
    );
    assert_eq!(
        git(
            &checkout,
            &[
                "show",
                &format!("{final_commit}:.ora-scheduled-artifacts/kept.md")
            ]
        ),
        "committed earlier"
    );
}

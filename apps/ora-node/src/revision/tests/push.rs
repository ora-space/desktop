//! Delivery pushes with real Git against the local Git Server double (Node delivery push ADR
//! D2–D4; Node clone ADR D8).
use super::*;
use crate::GitAccessConfig;
use crate::git_server::{GitServerAccess, RetryPolicy};
use gitlancer::CliGitRunner;
use ora_node_protocol::*;
use ora_test_support::git_server::GitServer;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::time::Duration;

const WORKSPACE: &str = "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a";

/// A Git Server repository and a checkout with a base commit and a final commit on top of it.
struct Setup {
    _directory: tempfile::TempDir,
    server: GitServer,
    repository: PathBuf,
    access: GitServerAccess,
    job: PushJob,
    base: CommitId,
    final_commit: CommitId,
}

fn setup() -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let server = GitServer::start(directory.path());
    let repository = server.create_repository("tenant/project.git");
    let root = directory.path().join("clones");
    std::fs::create_dir(&root).unwrap();
    let checkout = directory.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    git(&checkout, &["init", "-q", "-b", "main"]);
    git(&checkout, &["commit", "--allow-empty", "-qm", "base"]);
    let base = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
    git(&checkout, &["commit", "--allow-empty", "-qm", "work"]);
    let final_commit = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
    let material = server.access();
    let access = GitServerAccess::new(
        GitAccessConfig {
            ca_cert: material.ca_cert.clone(),
            client_cert: material.client_cert.clone(),
            client_key: material.client_key.clone(),
        },
        "/usr/bin:/bin".to_owned(),
        RetryPolicy {
            delays: [Duration::ZERO; 2],
        },
    );
    let job = PushJob {
        execution: ExecutionId::new("delivery"),
        spec: DeliverRevisionSpec {
            node_id: NodeId::new("node"),
            session_execution_id: ExecutionId::new("session"),
            checkout_execution_id: ExecutionId::new("clone"),
            base_commit: base.clone(),
            revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
            history_key: ObjectKey::new("runs/1/history.jsonl"),
            prior_revision: None,
            git_repository_url: GitRepositoryUrl::parse(&server.url("tenant/project.git")).unwrap(),
            delivery_ref: DeliveryRef::parse("refs/ora/deliveries/d-1").unwrap(),
            workspace_ref: WorkspaceRef::of(WORKSPACE, "main").unwrap(),
        },
        checkout,
        final_commit: final_commit.clone(),
        workspace_head: None,
        repository_root: root,
    };
    Setup {
        _directory: directory,
        server,
        repository,
        access,
        job,
        base,
        final_commit,
    }
}

impl Setup {
    /// Pushes `job` and returns how it ended.
    fn push(&self, job: &PushJob) -> Pushed {
        push(&CliGitRunner, &self.access, job, &|| false)
    }

    /// The Git Server's value of `name`, if it exists.
    fn remote(&self, name: &str) -> Option<String> {
        let listed = git(
            &self.repository,
            &["for-each-ref", "--format=%(objectname)", name],
        );
        (!listed.is_empty()).then_some(listed)
    }

    /// The same job pushing `commit` to `delivery_ref`.
    fn job(&self, commit: &CommitId, delivery_ref: &str, head: Option<&CommitId>) -> PushJob {
        let mut job = self.job.clone();
        job.final_commit = commit.clone();
        job.spec.delivery_ref = DeliveryRef::parse(delivery_ref).unwrap();
        job.workspace_head = head.cloned();
        job
    }
}

/// `delivered` and both kinds of `unchanged` push their final commit to their own delivery ref
/// and fast-forward the Workspace branch; a replay after a restart pushes the same commit again
/// without failing.
#[test]
fn every_outcome_is_pushed_and_a_replay_succeeds() {
    let setup = setup();
    let branch = setup.job.spec.workspace_ref.as_str().to_owned();
    // Unchanged at the base commit: the first push carries the base history.
    let unchanged = setup.job(&setup.base, "refs/ora/deliveries/unchanged-base", None);
    assert_eq!(
        setup.push(&unchanged),
        Pushed::Delivered {
            workspace_head: Some(setup.base.clone())
        }
    );
    let delivered = setup.job(
        &setup.final_commit,
        "refs/ora/deliveries/delivered",
        Some(&setup.base),
    );
    assert_eq!(
        setup.push(&delivered),
        Pushed::Delivered {
            workspace_head: Some(setup.final_commit.clone())
        }
    );
    // A replay finds both refs already at the commit (the ledger may have missed the branch).
    let replay = setup.job(
        &setup.final_commit,
        "refs/ora/deliveries/delivered",
        Some(&setup.base),
    );
    assert_eq!(setup.push(&replay), setup.push(&delivered));
    // Unchanged at a prior final commit the Git Server already has.
    let resumed = setup.job(
        &setup.final_commit,
        "refs/ora/deliveries/unchanged-prior",
        Some(&setup.final_commit),
    );
    assert_eq!(
        setup.push(&resumed),
        Pushed::Delivered {
            workspace_head: Some(setup.final_commit.clone())
        }
    );
    assert_eq!(
        [
            setup.remote("refs/ora/deliveries/unchanged-base"),
            setup.remote("refs/ora/deliveries/delivered"),
            setup.remote("refs/ora/deliveries/unchanged-prior"),
            setup.remote(&branch),
        ],
        [
            Some(setup.base.as_str().to_owned()),
            Some(setup.final_commit.as_str().to_owned()),
            Some(setup.final_commit.as_str().to_owned()),
            Some(setup.final_commit.as_str().to_owned()),
        ]
    );
    // Only the delivery ref and the Workspace branch were ever written.
    assert_eq!(
        git(&setup.repository, &["for-each-ref", "--format=%(refname)"])
            .lines()
            .filter(|name| !name.starts_with("refs/ora/deliveries/") && *name != branch)
            .count(),
        0
    );
}

/// A refused delivery ref fails at once as `push_rejected`; an unreachable Git Server is tried
/// three times and fails as `push_failed`.
#[test]
fn refused_and_unreachable_pushes_fail_with_their_codes() {
    let setup = setup();
    setup.server.answer_always(Some(403));
    let rejected = setup.push(&setup.job);
    let refused = setup.server.served().len();
    setup.server.answer_always(Some(503));
    let failed = setup.push(&setup.job);
    setup.server.answer_always(None);
    // A delivery ref that already holds another commit is refused by the Git Server.
    git(
        &setup.repository,
        &[
            "fetch",
            "-q",
            setup.job.checkout.to_str().unwrap(),
            &format!("{}:refs/ora/deliveries/taken", setup.base.as_str()),
        ],
    );
    let taken = setup.push(&setup.job(&setup.final_commit, "refs/ora/deliveries/taken", None));
    assert_eq!(
        [rejected, failed, taken],
        [
            Pushed::Failed(RevisionFailureCode::PushRejected),
            Pushed::Failed(RevisionFailureCode::PushFailed),
            Pushed::Failed(RevisionFailureCode::PushRejected),
        ]
    );
    assert_eq!(refused, 1);
    assert_eq!(
        setup
            .server
            .served()
            .iter()
            .filter(|request| request.status == 503)
            .count(),
        3
    );
}

/// A Workspace branch that moved elsewhere, or that the final commit does not descend from, is
/// left as it is while the delivery still succeeds.
#[test]
fn a_refused_workspace_fast_forward_never_fails_the_delivery() {
    let setup = setup();
    let branch = setup.job.spec.workspace_ref.as_str().to_owned();
    // Another push moved the branch to a commit the final commit does not contain.
    let other = setup.job.checkout.join("other");
    std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "-q"]);
    git(&other, &["commit", "--allow-empty", "-qm", "elsewhere"]);
    git(
        &setup.repository,
        &[
            "fetch",
            "-q",
            other.to_str().unwrap(),
            &format!("HEAD:{branch}"),
        ],
    );
    let elsewhere = setup.remote(&branch);
    // The ledger knows nothing: the declared absence is stale.
    let stale = setup.push(&setup.job(&setup.final_commit, "refs/ora/deliveries/stale", None));
    // The ledger knows the current value, but the final commit is not its descendant.
    let known = CommitId::new(elsewhere.clone().unwrap());
    let rewritten = setup.push(&setup.job(
        &setup.final_commit,
        "refs/ora/deliveries/rewritten",
        Some(&known),
    ));
    assert_eq!(
        [stale, rewritten],
        [
            Pushed::Delivered {
                workspace_head: None
            },
            Pushed::Delivered {
                workspace_head: None
            },
        ]
    );
    assert_eq!(setup.remote(&branch), elsewhere);
    assert_eq!(
        setup.remote("refs/ora/deliveries/rewritten"),
        Some(setup.final_commit.as_str().to_owned())
    );
}

//! Restoring with real Git against the local Git Server double (Node delivery push ADR D5, D6;
//! Node clone ADR D9).
use super::super::restore::RevisionRestorer;
use super::*;
use crate::GitAccessConfig;
use crate::git_server::{GitServerAccess, RetryPolicy};
use crate::session::{
    PriorRevisionRestore, RestoreCancel, RestoreFailure, RestoreRequest, Restored,
};
use gitlancer::{CliGitRunner, GitEnv};
use ora_node_protocol::*;
use ora_test_support::git_server::GitServer;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::time::Duration;

const DELIVERY: &str = "refs/ora/deliveries/d-1";

/// A Git Server repository holding a prior Revision's final commit at the delivery ref, and a
/// fresh checkout cloned at the snapshot `base` that does not have it.
struct Setup {
    _directory: tempfile::TempDir,
    server: GitServer,
    url: GitRepositoryUrl,
    root: PathBuf,
    checkout: PathBuf,
    /// The snapshot commit the checkout was cloned at.
    base: CommitId,
    /// The prior Revision's base, an ancestor of `base`.
    earlier: CommitId,
    /// The prior final commit, on top of `earlier`.
    prior: CommitId,
}

/// Commits an empty change in `directory` and returns it.
fn commit(directory: &Path, message: &str) -> CommitId {
    git(directory, &["commit", "--allow-empty", "-qm", message]);
    CommitId::new(git(directory, &["rev-parse", "HEAD"]))
}

fn setup() -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let server = GitServer::start(directory.path());
    let repository = server.create_repository("tenant/project.git");
    let url = GitRepositoryUrl::parse(&server.url("tenant/project.git")).unwrap();
    let root = directory.path().join("clones");
    std::fs::create_dir(&root).unwrap();
    // The prior run's checkout: its base, then its final commit.
    let source = directory.path().join("source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q"]);
    let earlier = commit(&source, "earlier");
    let prior = commit(&source, "prior work");
    git(
        &repository,
        &[
            "fetch",
            "-q",
            source.to_str().unwrap(),
            &format!("{}:{DELIVERY}", prior.as_str()),
        ],
    );
    // This Workspace's snapshot moved on from the prior base.
    git(
        &source,
        &["checkout", "-q", "-b", "snapshot", earlier.as_str()],
    );
    let base = commit(&source, "snapshot");
    let checkout = directory.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    git(&checkout, &["init", "-q", "-b", "main"]);
    git(
        &checkout,
        &[
            "fetch",
            "-q",
            "--update-head-ok",
            source.to_str().unwrap(),
            &format!("{}:refs/heads/main", base.as_str()),
        ],
    );
    git(&checkout, &["reset", "-q", "--hard"]);
    Setup {
        _directory: directory,
        server,
        url,
        root,
        checkout,
        base,
        earlier,
        prior,
    }
}

impl Setup {
    /// The production restorer over plain Git, with no pause between Git Server attempts.
    fn restorer(&self) -> RevisionRestorer<CliGitRunner, CliGitRunner> {
        let material = self.server.access();
        RevisionRestorer::new(
            DeliveryGit::new(
                CliGitRunner,
                GitPolicy {
                    env: GitEnv::automation_defaults()
                        .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                        .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                    config_args: vec!["-c".into(), "core.hooksPath=/dev/null".into()],
                },
            ),
            CliGitRunner,
            GitServerAccess::new(
                GitAccessConfig {
                    ca_cert: material.ca_cert.clone(),
                    client_cert: material.client_cert.clone(),
                    client_key: material.client_key.clone(),
                },
                "/usr/bin:/bin".to_owned(),
                RetryPolicy {
                    delays: [Duration::ZERO; 2],
                },
            ),
            self.root.clone(),
            /*owner*/ None,
            crate::Shutdown::default(),
        )
    }

    /// A restore of `final_commit` from `delivery_ref`, whose base was `prior_base`.
    fn request(
        &self,
        final_commit: &CommitId,
        delivery_ref: &str,
        prior_base: &CommitId,
    ) -> RestoreRequest {
        RestoreRequest {
            operation: OperationId::new("operation"),
            execution: ExecutionId::new("session"),
            checkout: self.checkout.clone(),
            branch: "main".to_owned(),
            base_commit: self.base.clone(),
            prior: PriorRevision {
                revision_id: RevisionId::new("revision"),
                final_commit: final_commit.clone(),
                git_repository_url: self.url.clone(),
                delivery_ref: DeliveryRef::parse(delivery_ref).unwrap(),
                base_commit: prior_base.clone(),
            },
        }
    }

    /// Restores with `restorer` and checks the transport repository is gone afterwards.
    async fn restore(
        &self,
        restorer: &RevisionRestorer<CliGitRunner, CliGitRunner>,
        request: RestoreRequest,
        cancel: RestoreCancel,
    ) -> Result<Restored, RestoreFailure> {
        let restored = restorer.restore(request, cancel).await;
        let left = std::fs::read_dir(self.root.join(".ora-transport"))
            .map(Iterator::count)
            .unwrap_or_default();
        assert_eq!(left, 0);
        restored
    }

    /// Where the checkout's Workspace branch points.
    fn head(&self) -> String {
        git(&self.checkout, &["rev-parse", "refs/heads/main"])
    }
}

/// The prior final commit lands on the Workspace branch; whether a note is due depends on the
/// prior base being in this checkout's snapshot history, and nothing but the branch, index and
/// worktree change.
#[tokio::test]
async fn restore_moves_the_branch_and_judges_divergence_by_the_snapshot() {
    let setup = setup();
    let restorer = setup.restorer();
    git(
        &setup.checkout,
        &[
            "update-ref",
            "refs/remotes/ora/upstream/main",
            setup.base.as_str(),
        ],
    );
    let remotes = git(&setup.checkout, &["for-each-ref", "refs/remotes/"]);
    let restored = setup
        .restore(
            &restorer,
            setup.request(&setup.prior, DELIVERY, &setup.earlier),
            RestoreCancel::default(),
        )
        .await;
    assert_eq!(restored, Ok(Restored::OnSnapshotHistory));
    assert_eq!(setup.head(), setup.prior.as_str());
    assert_eq!(
        git(&setup.checkout, &["symbolic-ref", "HEAD"]),
        "refs/heads/main"
    );
    assert_eq!(git(&setup.checkout, &["status", "--porcelain"]), "");
    assert_eq!(git(&setup.checkout, &["remote"]), "");
    assert_eq!(
        git(&setup.checkout, &["for-each-ref", "refs/remotes/"]),
        remotes
    );
    assert!(
        setup
            .server
            .served()
            .iter()
            .all(|request| request.execution_id.as_deref() == Some("session"))
    );
    // A prior base outside the snapshot's history (the external history was rewritten), or
    // one the checkout does not have at all, is noted.
    let unrelated = CommitId::new("e".repeat(40));
    for prior_base in [setup.prior.clone(), unrelated] {
        let restored = setup
            .restore(
                &restorer,
                setup.request(&setup.prior, DELIVERY, &prior_base),
                RestoreCancel::default(),
            )
            .await;
        assert_eq!(
            restored,
            Ok(Restored::Diverged {
                final_commit: setup.prior.clone(),
                base_commit: prior_base,
                branch: "main".to_owned(),
            })
        );
    }
}

/// A missing delivery ref and a refused request are permanent; another commit at the ref and
/// an unreachable server are retryable. None moves the branch.
#[tokio::test]
async fn restore_failures_follow_the_git_server_answer() {
    let setup = setup();
    let restorer = setup.restorer();
    let missing = setup
        .restore(
            &restorer,
            setup.request(&setup.prior, "refs/ora/deliveries/absent", &setup.earlier),
            RestoreCancel::default(),
        )
        .await;
    setup.server.answer_always(Some(403));
    let denied = setup
        .restore(
            &restorer,
            setup.request(&setup.prior, DELIVERY, &setup.earlier),
            RestoreCancel::default(),
        )
        .await;
    setup.server.answer_always(Some(503));
    let unavailable = setup
        .restore(
            &restorer,
            setup.request(&setup.prior, DELIVERY, &setup.earlier),
            RestoreCancel::default(),
        )
        .await;
    setup.server.answer_always(None);
    let mismatch = setup
        .restore(
            &restorer,
            setup.request(&setup.earlier, DELIVERY, &setup.earlier),
            RestoreCancel::default(),
        )
        .await;
    assert_eq!(
        [missing, denied, unavailable, mismatch],
        [
            Err(RestoreFailure::BaseUnavailable),
            Err(RestoreFailure::BaseUnavailable),
            Err(RestoreFailure::Unavailable),
            Err(RestoreFailure::Unavailable),
        ]
    );
    assert_eq!(setup.head(), setup.base.as_str());
    let attempts = setup
        .server
        .served()
        .iter()
        .filter(|request| request.status == 503)
        .count();
    assert_eq!(attempts, 3);
}

/// A cancelled session and a spent deadline start no Git Server request and leave the branch.
#[tokio::test]
async fn cancelled_or_expired_restores_reach_nothing() {
    let setup = setup();
    let cancel = RestoreCancel::default();
    cancel.cancel();
    let cancelled = setup
        .restore(
            &setup.restorer(),
            setup.request(&setup.prior, DELIVERY, &setup.earlier),
            cancel,
        )
        .await;
    let expired = setup
        .restore(
            &setup.restorer().within(Duration::ZERO),
            setup.request(&setup.prior, DELIVERY, &setup.earlier),
            RestoreCancel::default(),
        )
        .await;
    assert_eq!(
        [cancelled, expired],
        [
            Err(RestoreFailure::Unavailable),
            Err(RestoreFailure::Unavailable)
        ]
    );
    assert_eq!(setup.server.served(), Vec::new());
    assert_eq!(setup.head(), setup.base.as_str());
}

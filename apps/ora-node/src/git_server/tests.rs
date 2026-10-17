#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use gitlancer::CliGitRunner;
use ora_test_support::git_server::{GitServer, git};
use pretty_assertions::assert_eq;
use std::path::Path;

/// A Git Server with one repository holding two commits, and a Node-side access to it.
struct Setup {
    _directory: tempfile::TempDir,
    root: PathBuf,
    server: GitServer,
    url: GitRepositoryUrl,
    access: GitServerAccess,
    /// A non-bare checkout holding both commits, with `main` at the second.
    checkout: PathBuf,
    first: CommitId,
    second: CommitId,
}

use std::path::PathBuf;

/// Publishes a two-commit history as `refs/heads/workspaces/w/main`.
fn setup() -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repositories");
    std::fs::create_dir(&root).unwrap();
    let server = GitServer::start(directory.path());
    server.create_repository("tenant/project.git");
    let url = GitRepositoryUrl::parse(&server.url("tenant/project.git")).unwrap();
    let checkout = directory.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    git(&checkout, &["init", "--quiet", "-b", "main"]);
    let commit = |message: &str| {
        git(
            &checkout,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.test",
                "commit",
                "--allow-empty",
                "-qm",
                message,
            ],
        );
        CommitId::new(git(&checkout, &["rev-parse", "HEAD"]))
    };
    let first = commit("one");
    let second = commit("two");
    let material = server.access();
    let access = GitServerAccess::new(
        GitAccessConfig {
            ca_cert: material.ca_cert.clone(),
            client_cert: material.client_cert.clone(),
            client_key: material.client_key.clone(),
        },
        "/usr/bin:/bin".to_owned(),
        RetryPolicy {
            delays: [Duration::from_millis(10), Duration::from_millis(10)],
        },
    );
    Setup {
        _directory: directory,
        root,
        server,
        url,
        access,
        checkout,
        first,
        second,
    }
}

/// Never stopping, for tests that do not exercise a Node stop.
fn running() -> bool {
    false
}

impl Setup {
    /// A transport repository borrowing the checkout's objects.
    fn transport(&self, kind: &str) -> TransportRepository {
        let transport =
            TransportRepository::create(&CliGitRunner, &self.root, kind, &execution()).unwrap();
        transport
            .borrow_objects(&self.checkout.join(".git"))
            .unwrap();
        transport
    }

    /// Pushes `commit` to `destination` with `lease`.
    fn push(
        &self,
        transport: &TransportRepository,
        commit: &CommitId,
        destination: &str,
        lease: Lease,
    ) -> Result<PushReport, GitServerFailure> {
        self.access.push(
            &CliGitRunner,
            transport,
            &self.url,
            &execution(),
            &RefUpdate {
                commit,
                destination,
                lease,
            },
            &running,
        )
    }
}

/// The execution every request in these tests declares.
fn execution() -> ExecutionId {
    ExecutionId::new("execution-1")
}

const BRANCH: &str = "refs/heads/workspaces/w/main";

/// A push reports creation, replay, stale leases and declined updates distinctly, and every
/// request carries the execution header.
#[test]
fn push_reports_each_fate_of_the_ref() {
    let setup = setup();
    let transport = setup.transport("push");
    assert_eq!(
        [
            setup.push(&transport, &setup.first, BRANCH, Lease::Absent),
            setup.push(&transport, &setup.first, BRANCH, Lease::Absent),
            setup.push(
                &transport,
                &setup.second,
                BRANCH,
                Lease::At(setup.second.clone())
            ),
            setup.push(
                &transport,
                &setup.second,
                BRANCH,
                Lease::At(setup.first.clone())
            ),
            setup.push(
                &transport,
                &setup.first,
                BRANCH,
                Lease::At(setup.second.clone())
            ),
            setup.push(
                &transport,
                &setup.first,
                "refs/heads/main",
                Lease::Unchecked
            ),
        ],
        [
            Ok(PushReport::Updated),
            // A replay of the same commit is up to date even though the declared old value is
            // stale: Git compares the values before the lease.
            Ok(PushReport::UpToDate),
            Ok(PushReport::Stale),
            Ok(PushReport::Updated),
            Ok(PushReport::Rejected),
            Ok(PushReport::Rejected),
        ]
    );
    assert_eq!(
        setup.push(&transport, &setup.second, BRANCH, Lease::Unchecked),
        Ok(PushReport::UpToDate)
    );
    assert!(
        setup
            .server
            .served()
            .iter()
            .all(|request| request.execution_id.as_deref() == Some("execution-1"))
    );
}

/// 403 is not retried; 503 is retried until it succeeds or the attempts run out.
#[test]
fn denied_requests_fail_at_once_and_unavailable_ones_are_retried() {
    let setup = setup();
    let transport = setup.transport("retry");
    setup.server.fail_next(403, 1);
    assert_eq!(
        setup.push(&transport, &setup.first, BRANCH, Lease::Absent),
        Err(GitServerFailure::Denied)
    );
    assert_eq!(setup.server.served().len(), 1);
    setup.server.fail_next(503, 2);
    assert_eq!(
        setup.push(&transport, &setup.first, BRANCH, Lease::Absent),
        Ok(PushReport::Updated)
    );
    setup.server.answer_always(Some(503));
    let before = setup.server.served().len();
    let fetched = setup.access.fetch(
        &CliGitRunner,
        &transport,
        &setup.url,
        &execution(),
        &[format!("{BRANCH}:refs/ora/fetched")],
        &running,
    );
    assert_eq!(fetched, Err(GitServerFailure::Unavailable));
    assert_eq!(setup.server.served().len() - before, 3);
    setup.server.answer_always(None);
    let before_stop = setup.server.served().len();
    let stopped = setup.access.fetch(
        &CliGitRunner,
        &transport,
        &setup.url,
        &execution(),
        &[format!("{BRANCH}:refs/ora/fetched")],
        &|| true,
    );
    assert_eq!(stopped, Err(GitServerFailure::Stopped));
    assert_eq!(setup.server.served().len(), before_stop);
}

/// A missing ref is its own answer, and fetched refs land only in the transport repository.
#[test]
fn fetch_reads_into_the_transport_and_names_missing_refs() {
    let setup = setup();
    let transport = setup.transport("fetch");
    setup
        .push(&transport, &setup.second, BRANCH, Lease::Absent)
        .unwrap();
    let fresh =
        TransportRepository::create(&CliGitRunner, &setup.root, "fresh", &execution()).unwrap();
    setup
        .access
        .fetch(
            &CliGitRunner,
            &fresh,
            &setup.url,
            &execution(),
            &[format!("{BRANCH}:refs/ora/input")],
            &running,
        )
        .unwrap();
    assert_eq!(
        fresh.commit_of(&CliGitRunner, "refs/ora/input").unwrap(),
        Some(setup.second.clone())
    );
    assert_eq!(
        setup.access.fetch(
            &CliGitRunner,
            &fresh,
            &setup.url,
            &execution(),
            &["refs/heads/workspaces/w/absent:refs/ora/input".to_owned()],
            &running,
        ),
        Err(GitServerFailure::MissingRef)
    );
    assert_eq!(
        setup
            .access
            .remote_refs(
                &CliGitRunner,
                &fresh,
                &setup.url,
                &execution(),
                &["refs/heads/workspaces/*".to_owned()],
                &running,
            )
            .unwrap(),
        vec![(BRANCH.to_owned(), setup.second.clone())]
    );
    let path = fresh.path().to_path_buf();
    drop(fresh);
    assert!(!path.exists());
}

/// Settings an Agent writes into its checkout never reach Node-identity Git: a credential
/// helper, a URL rewrite and a per-URL TLS override are all ignored, and a checkout that
/// borrows objects from elsewhere is not borrowed from.
#[test]
fn checkout_configuration_has_no_effect_on_node_identity_git() {
    let setup = setup();
    let marker = setup.checkout.join("helper-ran");
    let helper = format!("!f() {{ touch {}; }}; f", marker.display());
    for (key, value) in [
        ("credential.helper", helper.as_str()),
        ("core.askPass", "/bin/false"),
        (&format!("http.{}.sslVerify", setup.url.as_str()), "false"),
        (
            &format!("url.https://example.invalid/.insteadOf"),
            "https://localhost",
        ),
    ] {
        git(&setup.checkout, &["config", key, value]);
    }
    let transport = setup.transport("hostile");
    assert_eq!(
        setup.push(&transport, &setup.second, BRANCH, Lease::Absent),
        Ok(PushReport::Updated)
    );
    assert!(!marker.exists());
    let info = setup.checkout.join(".git").join("objects").join("info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("alternates"), "/elsewhere/objects\n").unwrap();
    let other =
        TransportRepository::create(&CliGitRunner, &setup.root, "other", &execution()).unwrap();
    assert!(other.borrow_objects(&setup.checkout.join(".git")).is_err());
    assert!(!Path::new(&other.path().join("objects").join("info").join("alternates")).exists());
}

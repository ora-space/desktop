use super::support::Fixture;
use ora_node::{CloneConfig, Node, Shutdown};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::atomic::Ordering};

#[path = "https.rs"]
mod https;
use https::GitServerFixture;

#[path = "repository_recovery.rs"]
mod recovery;

#[path = "repository_policy.rs"]
mod policy;

#[path = "repository_commit.rs"]
mod commit;

#[path = "repository_ipc.rs"]
mod ipc;

#[path = "repository_controller.rs"]
mod controller;

#[path = "repository_session.rs"]
mod session;

#[path = "repository_takeover_crash.rs"]
mod takeover_crash;

#[path = "repository_single_node.rs"]
mod single_node;

#[path = "repository_runtime_fixture.rs"]
mod runtime_fixture;
#[path = "repository_websocket.rs"]
mod websocket;

#[path = "repository_busy_reading.rs"]
mod busy_reading;

#[path = "repository_admission.rs"]
mod admission;
#[path = "agent_sessions.rs"]
mod agent_sessions;
#[path = "repository_plugins.rs"]
mod plugins;

/// Provides an explicit deployment Git configuration without modifying process environment or
/// user Git config; the Node reaches the Git Server with its git-access identity instead.
fn configuration(fixture: &Fixture, server: &GitServerFixture) -> CloneConfig {
    let root = fixture.path().join("clones");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
    let git_config = fixture.path().join("clone.gitconfig");
    fs::write(
        &git_config,
        format!(
            "[http]\n sslCAInfo = {}\n",
            server.access().ca_cert.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&git_config, fs::Permissions::from_mode(/*mode*/ 0o600)).unwrap();
    CloneConfig {
        repository_root: root,
        git_config,
        search_path: vec![PathBuf::from("/usr/bin")],
    }
}

/// Clones the published snapshot onto Workspace `branch`, with explicit operation and execution
/// identity; target selection stays with the Node.
fn request(server: &GitServerFixture, suffix: &str, branch: &str) -> CloneRepositoryMessage {
    snapshot_request(server, suffix, &server.snapshot(), branch)
}

/// Clones snapshot `commit` onto Workspace `branch`.
fn snapshot_request(
    server: &GitServerFixture,
    suffix: &str,
    commit: &CommitId,
    branch: &str,
) -> CloneRepositoryMessage {
    CloneRepositoryMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: OperationId::new(format!("clone-op-{suffix}")),
        execution_id: ExecutionId::new(format!("clone-exec-{suffix}")),
        payload: CloneRepository {
            spec: server.spec("test-node", commit, branch),
        },
    }
}

/// Runs read-only Git in a checkout for assertions.
fn checkout_git(path: &std::path::Path, args: &[&str]) -> String {
    ora_test_support::git_server::git(path, args)
}

/// Uses actual TLS, Git, host and guardian: the snapshot is fetched as the Node, the checkout is
/// built from it on the Workspace branch with no remote, and a replay needs no network.
#[test]
fn managed_snapshot_clone_checks_out_the_base_commit_without_a_remote() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["checkout", "-b", "feature/clone"]);
        fs::write(
            fixture.path().join("main").join("branch.txt"),
            "snapshot content",
        )
        .unwrap();
        fixture.git(&["add", "branch.txt"]);
        fixture.git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "selected",
        ]);
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main"));
        let commit = server.snapshot();
        assert_eq!(commit.as_str(), fixture.git(&["rev-parse", "HEAD"]));
        fixture.git(&["checkout", "main"]);
        let config = configuration(&fixture, &server);
        let mut node_config = fixture.config();
        node_config.repositories.clear();
        let mut node = Node::open(node_config, fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
        node.configure_git_access(server.access());
        let command = request(&server, "success", "main");
        let result = node.submit_clone(command.clone()).unwrap();
        let ExecutionState::Completed(ExecutionResult::Clone(clone)) = &result.state else {
            panic!("clone not completed: {result:?}");
        };
        let CloneExecutionResult::CloneReady(ready) = clone.as_ref() else {
            panic!("clone not ready: {result:?}");
        };
        assert_eq!(ready.commit, commit);
        assert_eq!(ready.spec, command.payload.spec);
        let checkout = PathBuf::from(ready.path.as_str());
        assert_eq!(
            fs::read_to_string(checkout.join("branch.txt")).unwrap(),
            "snapshot content"
        );
        // The checkout is on the Workspace branch at the base commit, with nothing pointing
        // anywhere: no remote, no remote-tracking ref, no credential or transport setting.
        assert_eq!(checkout_git(&checkout, &["remote"]), "");
        assert_eq!(
            checkout_git(&checkout, &["symbolic-ref", "HEAD"]),
            "refs/heads/main"
        );
        assert_eq!(
            checkout_git(&checkout, &["rev-parse", "HEAD"]),
            commit.as_str()
        );
        assert_eq!(
            checkout_git(&checkout, &["for-each-ref", "refs/remotes/"]),
            ""
        );
        let settings = checkout_git(&checkout, &["config", "--local", "--list"]);
        assert!(
            !settings
                .lines()
                .any(|line| ["remote.", "http.", "credential."]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))),
            "{settings}"
        );
        assert!(!checkout.join(".git/objects/info/alternates").exists());
        assert_eq!(
            checkout_git(&checkout, &["rev-parse", "--is-shallow-repository"]),
            "false"
        );
        // The transport repository is gone once the clone ended.
        assert_eq!(
            fs::read_dir(config.repository_root.join(".ora-transport"))
                .map(Iterator::count)
                .unwrap_or_default(),
            0
        );
        // Every request declared the clone execution.
        assert!(
            server
                .server
                .served()
                .iter()
                .all(|request| request.execution_id.as_deref() == Some("clone-exec-success"))
        );
        server.reject_auth(true);
        fs::write(checkout.join("branch.txt"), "user edits").unwrap();
        assert_eq!(node.submit_clone(command.clone()).unwrap(), result);
        let events = node.pending_events().unwrap();
        drop(node);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        assert_eq!(node.submit_clone(command).unwrap().state, result.state);
        assert_eq!(node.pending_events().unwrap(), events);
        assert_eq!(
            fs::read_to_string(checkout.join("branch.txt")).unwrap(),
            "user edits"
        );
    });
}

/// A missing or unreadable snapshot ref, and one that points at another commit, fail with their
/// own codes and retain separate destinations; a new execution uses a new directory.
#[test]
fn snapshot_failures_retain_targets_and_new_retry_uses_new_directory() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main"));
        let config = configuration(&fixture, &server);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        let failure = |state: &ExecutionState| match state {
            ExecutionState::Completed(ExecutionResult::Clone(result)) => match result.as_ref() {
                CloneExecutionResult::CloneFailed(failed) => Some(failed.clone()),
                CloneExecutionResult::CloneReady(_) => None,
            },
            _ => None,
        };
        let absent = CommitId::new("1".repeat(40));
        let missing = node
            .submit_clone(snapshot_request(&server, "missing", &absent, "main"))
            .unwrap();
        let missing = failure(&missing.state).unwrap();
        assert_eq!(missing.failure, CloneFailureCode::SnapshotUnavailable);
        let CloneResidual::Retained { path: old, .. } = &missing.residual else {
            panic!("missing residue");
        };
        assert!(PathBuf::from(old.as_str()).is_dir());
        server.reject_auth(true);
        let denied = node.submit_clone(request(&server, "auth", "main")).unwrap();
        assert_eq!(
            failure(&denied.state).map(|failed| failed.failure),
            Some(CloneFailureCode::SnapshotUnavailable)
        );
        server.reject_auth(false);
        // A snapshot ref that names one commit but points at another is a data mismatch.
        let other = fixture.git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "--allow-empty",
            "-m",
            "other",
        ]);
        let _ = other;
        let head = CommitId::new(fixture.git(&["rev-parse", "HEAD"]));
        let snapshot = server.snapshot();
        ora_test_support::git_server::git(
            &server.repository,
            &[
                "update-ref",
                &format!(
                    "refs/ora/upstreams/{}/snapshots/{}",
                    https::BINDING,
                    head.as_str()
                ),
                snapshot.as_str(),
            ],
        );
        let mismatch = node
            .submit_clone(snapshot_request(&server, "mismatch", &head, "main"))
            .unwrap();
        assert_eq!(
            failure(&mismatch.state).map(|failed| failed.failure),
            Some(CloneFailureCode::BaseCommitMismatch)
        );
        let retry = node
            .submit_clone(request(&server, "retry", "main"))
            .unwrap();
        let ExecutionState::Completed(ExecutionResult::Clone(result)) = retry.state else {
            panic!("retry not completed");
        };
        let CloneExecutionResult::CloneReady(ready) = *result else {
            panic!("retry not ready");
        };
        assert_ne!(ready.path, *old);
        assert!(PathBuf::from(old.as_str()).is_dir());
        assert_eq!(node.pending_events().unwrap().len(), 4);
    });
}

/// A 503 is retried within the same pass and the clone then succeeds.
#[test]
fn unavailable_git_server_is_retried_within_the_fetch_pass() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main"));
        let config = configuration(&fixture, &server);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        server.server.fail_next(503, 1);
        let state = node
            .submit_clone(request(&server, "unavailable", "main"))
            .unwrap()
            .state;
        assert!(
            matches!(&state, ExecutionState::Completed(ExecutionResult::Clone(result)) if matches!(result.as_ref(), CloneExecutionResult::CloneReady(_))),
            "{state:?}"
        );
        assert_eq!(server.server.served()[0].status, 503);
    });
}

/// A Git Server that keeps answering 503 is tried three times in the pass; the clone then ends
/// as `SourceUnavailable` and keeps its directory.
#[test]
fn unavailable_git_server_fails_the_clone_once_the_attempts_run_out() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main"));
        let config = configuration(&fixture, &server);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        node.set_git_server_retry_delays([std::time::Duration::ZERO; 2]);
        server.server.answer_always(Some(503));
        let state = node
            .submit_clone(request(&server, "exhausted", "main"))
            .unwrap()
            .state;
        let Some(CloneExecutionResult::CloneFailed(failed)) = clone_result(&state) else {
            panic!("clone did not fail: {state:?}");
        };
        assert_eq!(failed.failure, CloneFailureCode::SourceUnavailable);
        let CloneResidual::Retained { path, .. } = &failed.residual else {
            panic!("missing residue: {failed:?}");
        };
        assert!(PathBuf::from(path.as_str()).is_dir());
        assert_eq!(
            server
                .server
                .served()
                .iter()
                .map(|request| request.status)
                .collect::<Vec<_>>(),
            vec![503; 3]
        );
    });
}

/// Historical SQLite dispatch evidence for recovery tests. Local clone intake was removed; this
/// fixture writes only the pre-existing responsibility that the reconciliation adapter still owns.
#[derive(Debug, PartialEq, Eq)]
struct HistoricalClone {
    command: CloneRepositoryMessage,
    result: Option<CloneExecutionResult>,
}

/// Seeds an original dispatch while the test owns the initialized local store.
fn seed_dispatch(
    home: &std::path::Path,
    mut command: CloneRepositoryMessage,
    request_id: RequestId,
) -> CloneRepositoryMessage {
    command.request_id = Some(request_id.clone());
    rusqlite::Connection::open(home.join("ora-controller.sqlite3")).unwrap().execute(
        "INSERT INTO clone_operations(request,operation,execution,node,input) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![request_id.as_str(), command.operation_id.as_str(), command.execution_id.as_str(), command.payload.spec.node_id.as_str(), serde_json::to_string(&command).unwrap()],
    ).unwrap();
    command
}

/// Reads retained dispatch and result evidence without restoring the retired local intake API.
fn historical_clones(home: &std::path::Path) -> Vec<HistoricalClone> {
    let connection = rusqlite::Connection::open(home.join("ora-controller.sqlite3")).unwrap();
    let mut statement = connection
        .prepare("SELECT input,result FROM clone_operations ORDER BY rowid")
        .unwrap();
    statement
        .query_map([], |row| {
            let input: String = row.get(0)?;
            let result: Option<String> = row.get(1)?;
            Ok(HistoricalClone {
                command: serde_json::from_str(&input).unwrap(),
                result: result.map(|s| serde_json::from_str(&s).unwrap()),
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Extracts clone evidence while keeping the protocol's boxed envelope at its boundary.
fn clone_result(state: &ExecutionState) -> Option<&CloneExecutionResult> {
    match state {
        ExecutionState::Completed(ExecutionResult::Clone(result)) => Some(result.as_ref()),
        _ => None,
    }
}

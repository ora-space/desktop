use super::support::Fixture;
use ora_node::{CloneConfig, CloneSsh, Node, Shutdown};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::atomic::Ordering};

#[path = "https.rs"]
mod https;
use https::HttpsRepository;

#[path = "repository_recovery.rs"]
mod recovery;

#[path = "repository_policy.rs"]
mod policy;

#[path = "repository_ssh.rs"]
mod ssh;

#[path = "repository_commit.rs"]
mod commit;

/// Provides explicit trusted TLS configuration without modifying process environment or user Git config.
fn configuration(fixture: &Fixture, server: &HttpsRepository) -> CloneConfig {
    let root = fixture.path().join("clones");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
    let git_config = fixture.path().join("clone.gitconfig");
    fs::write(
        &git_config,
        format!("[http]\n sslCAInfo = {}\n", server.certificate.display()),
    )
    .unwrap();
    fs::set_permissions(&git_config, fs::Permissions::from_mode(/*mode*/ 0o600)).unwrap();
    CloneConfig {
        repository_root: root,
        git_config,
        search_path: vec![PathBuf::from("/usr/bin")],
        ssh: CloneSsh::Disabled,
    }
}

/// Makes operation/execution identity explicit while leaving target selection exclusively to Node.
fn request(server: &HttpsRepository, suffix: &str, branch: &str) -> CloneRepositoryMessage {
    CloneRepositoryMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: OperationId::new(format!("clone-op-{suffix}")),
        execution_id: ExecutionId::new(format!("clone-exec-{suffix}")),
        payload: CloneRepository {
            spec: CloneExecutionSpec {
                node_id: NodeId::new("test-node"),
                repository: CloneRepositoryUrl::parse(&server.address).unwrap(),
                branch: BranchName::new(branch),
            },
        },
    }
}

/// Uses actual TLS, Git, host and guardian; a local-path clone cannot satisfy this acceptance.
#[test]
fn managed_https_clone_selects_branch_and_replays_without_network() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["checkout", "-b", "feature/clone"]);
        fs::write(
            fixture.path().join("main").join("branch.txt"),
            "selected branch",
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
        let commit = fixture.git(&["rev-parse", "HEAD"]);
        fixture.git(&["checkout", "main"]);
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let mut node_config = fixture.config();
        node_config.repositories.clear();
        let mut node = Node::open(node_config, fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        let command = request(&server, "success", "feature/clone");
        let result = node.submit_clone(command.clone()).unwrap();
        let ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneReady(
            ready,
        ))) = &result.state
        else {
            panic!("clone not ready: {result:?}");
        };
        assert_eq!(ready.commit, CommitId::new(commit));
        assert_eq!(ready.spec, command.payload.spec);
        assert_eq!(
            fs::read_to_string(PathBuf::from(ready.path.as_str()).join("branch.txt")).unwrap(),
            "selected branch"
        );
        server.reject_auth.store(true, Ordering::SeqCst);
        fs::write(
            PathBuf::from(ready.path.as_str()).join("branch.txt"),
            "user edits",
        )
        .unwrap();
        assert_eq!(node.submit_clone(command.clone()).unwrap(), result);
        let events = node.pending_events().unwrap();
        drop(node);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        assert_eq!(node.submit_clone(command).unwrap().state, result.state);
        assert_eq!(node.pending_events().unwrap(), events);
        assert_eq!(
            fs::read_to_string(PathBuf::from(ready.path.as_str()).join("branch.txt")).unwrap(),
            "user edits"
        );
    });
}

/// Missing branches and noninteractive authentication fail explicitly and retain separate destinations.
#[test]
fn managed_https_failures_retain_targets_and_new_retry_uses_new_directory() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["tag", "only-tag"]);
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        let tag = node
            .submit_clone(request(&server, "tag", "only-tag"))
            .unwrap();
        assert!(matches!(
            tag.state,
            ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneFailed(
                CloneFailed {
                    failure: CloneFailureCode::BranchNotFound,
                    ..
                }
            )))
        ));
        let missing = node
            .submit_clone(request(&server, "missing", "absent"))
            .unwrap();
        let ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneFailed(
            failed,
        ))) = &missing.state
        else {
            panic!("missing branch not failed: {missing:?}");
        };
        let CloneResidual::Retained { path: old, .. } = &failed.residual else {
            panic!("missing residue");
        };
        assert!(PathBuf::from(old.as_str()).is_dir());
        server.reject_auth.store(true, Ordering::SeqCst);
        let auth = node.submit_clone(request(&server, "auth", "main")).unwrap();
        assert!(matches!(
            auth.state,
            ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneFailed(_)))
        ));
        server.reject_auth.store(false, Ordering::SeqCst);
        let retry = node
            .submit_clone(request(&server, "retry", "main"))
            .unwrap();
        let ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneReady(
            ready,
        ))) = retry.state
        else {
            panic!("retry not ready");
        };
        assert_ne!(ready.path, *old);
        assert!(PathBuf::from(old.as_str()).is_dir());
        assert_eq!(node.pending_events().unwrap().len(), 4);
    });
}

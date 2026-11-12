use super::*;
use crate::support::{ChildGuard, until};
use ora_node_db::{CloneExecution, CloneTarget, NodeDatabase};
use pretty_assertions::assert_eq;
use std::process::{Command, Stdio};

/// Seeds the same durable acceptance interface used before the production runtime creates a directory.
fn seed(
    fixture: &Fixture,
    config: &CloneConfig,
    command: &CloneRepositoryMessage,
) -> CloneExecution {
    drop(Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap());
    let mut database = NodeDatabase::open(
        &fixture.config().home_directory.join("ora-node.sqlite3"),
        fixture.config().identity,
    )
    .unwrap();
    database
        .accept_clone(
            command,
            &CloneTarget {
                repository_id: RepositoryId::new("reserved-repository"),
                root: config.repository_root.clone(),
                path: config.repository_root.join("reserved-repository"),
            },
        )
        .unwrap()
}

/// Runs the real recovery executable; a config file supplies deployment settings, never commands.
fn launch(fixture: &Fixture, config: &CloneConfig, label: &str) -> ChildGuard {
    let path = fixture.path().join(format!("{label}.json"));
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "node": fixture.config(), "process": fixture.process(), "clone": config,
            "git_access": { "ca_cert": fixture.path().join("git-server/git-access/ca.pem"),
                "client_cert": fixture.path().join("git-server/git-access/client.pem"),
                "client_key": fixture.path().join("git-server/git-access/client-key.pem") },
            "timezone": "Asia/Shanghai", "recovery_interval_ms": 100,
        }))
        .unwrap(),
    )
    .unwrap();
    ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_ora-node"))
            .arg(path)
            .stdin(Stdio::null())
            .stdout(fs::File::create(fixture.path().join(format!("{label}.log"))).unwrap())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}

/// A directory created before ownership evidence commits is not adopted, even when empty.
#[test]
fn accepted_clone_never_adopts_an_existing_directory_or_replaced_root() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "existing", "main");
        let record = seed(&fixture, &config, &command);
        fs::create_dir(&record.target.path).unwrap();
        let sentinel = record.target.path.join("user.txt");
        fs::write(&sentinel, "preserve").unwrap();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
        node.configure_git_access(server.access());
        assert_eq!(
            node.recover_clones().unwrap(),
            ora_node::NodeState::RecoveryPending
        );
        assert_eq!(
            node.submit_clone(command).unwrap().state,
            ExecutionState::Unknown
        );
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "preserve");
        assert!(node.pending_events().unwrap().is_empty());

        let moved = fixture.path().join("moved-clones");
        fs::rename(&config.repository_root, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &config.repository_root).unwrap();
        let second = request(&server, "replaced-root", "main");
        assert_eq!(
            node.submit_clone(second).unwrap().state,
            ExecutionState::Unknown
        );
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    });
}

/// Killing a real Node during the workload checkout fetch cannot authorize another attempt or a
/// directory-based success: the restarted Node settles the original attempt before it recovers.
#[test]
fn killed_clone_recovers_original_attempt_without_redispatch() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let gate = https::WorkloadGate::install(fixture.path());
        let command = request(&server, "kill", "main");
        let record = seed(&fixture, &config, &command);
        let mut child = launch(&fixture, &config, "clone-first");
        until(|| gate.ready.exists());
        child.kill();
        let mut replacement = launch(&fixture, &config, "clone-replacement");
        until(|| {
            fixture
                .log("clone-replacement")
                .contains("Node recovery pass completed")
        });
        replacement.terminate();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        let state = node.submit_clone(command.clone()).unwrap().state;
        // Nothing of the old Git was left when recovery took its attempt lock, so the attempt is
        // found stopped without an exit status: a retryable failure, not success and not
        // permanent Unknown.
        assert_interrupted(&state, &record, &command);
        drop(node);
        assert_retained_attempts(&fixture, &record, &command, 2);
    });
}

/// A normal stop settles the clone it cut short before exiting; no later recovery is needed.
#[test]
fn stopped_node_settles_its_interrupted_clone_before_exit() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let gate = https::WorkloadGate::install(fixture.path());
        let command = request(&server, "stop", "main");
        let record = seed(&fixture, &config, &command);
        let mut child = launch(&fixture, &config, "clone-stopped");
        until(|| gate.ready.exists());
        child.terminate();
        // Reading without recover_clones proves the stopping process itself persisted the result.
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        let state = node.submit_clone(command.clone()).unwrap().state;
        assert_interrupted(&state, &record, &command);
        drop(node);
        assert_retained_attempts(&fixture, &record, &command, 2);
    });
}

/// Slow network work can outlive the local-command budget without being interrupted.
#[test]
fn slow_clone_outlives_the_local_command_budget() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::for_command_deadline();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.pause(true);
        let config = configuration(&fixture, &server);
        let command = request(&server, "slow-network", "main");
        let record = seed(&fixture, &config, &command);
        let pause = server.pause_flag();
        let target = record.target.path.clone();
        let release = std::thread::spawn(move || {
            until(|| target.is_dir());
            std::thread::sleep(std::time::Duration::from_secs(7));
            pause.store(false, Ordering::SeqCst);
        });
        let mut process = fixture.process();
        // Keep local Git viable on loaded CI machines, while proving
        // that the seven-second network wait exceeds the unchanged five-second local budget.
        process.command_timeout_ms = 5_000;
        process.network_timeout_ms = 30_000;
        let mut node = Node::open(fixture.config(), process, Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        let state = node.submit_clone(command.clone()).unwrap().state;
        release.join().unwrap();
        assert!(
            matches!(
                clone_result(&state),
                Some(CloneExecutionResult::CloneReady(_))
            ),
            "slow network clone failed: {state:?}"
        );
        drop(node);
        assert_retained_attempts(&fixture, &record, &command, 3);
    });
}

/// An expired network deadline ends the attempt the same way a stop does.
#[test]
fn command_deadline_settles_clone_as_interrupted() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::for_command_deadline();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let _gate = https::WorkloadGate::install(fixture.path());
        let command = request(&server, "deadline", "main");
        let record = seed(&fixture, &config, &command);
        let mut process = fixture.process();
        // A dispatched clone step waits up to the network budget; the Node's own local Git in
        // the transport repository keeps the ordinary local budget.
        process.command_timeout_ms = 10_000;
        process.network_timeout_ms = 4_000;
        let mut node = Node::open(fixture.config(), process, Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        let state = node.submit_clone(command.clone()).unwrap().state;
        assert_eq!(
            state,
            ExecutionState::Completed(ExecutionResult::Clone(Box::new(
                CloneExecutionResult::CloneFailed(CloneFailed {
                    node: node.identity().clone(),
                    spec: command.payload.spec.clone(),
                    failure: CloneFailureCode::Interrupted,
                    residual: retained(&record),
                })
            )))
        );
        drop(node);
        assert_retained_attempts(&fixture, &record, &command, 2);
    });
}

/// While the attempt lock is still held the old Git may still write, so nothing is concluded;
/// once that Git has ended, the same attempt settles as interrupted.
#[test]
fn clone_stays_unknown_while_its_git_outlives_the_node() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let gate = https::WorkloadGate::install(fixture.path());
        let command = request(&server, "lost", "main");
        let record = seed(&fixture, &config, &command);
        let mut child = launch(&fixture, &config, "clone-lost");
        until(|| gate.ready.exists());
        fixture.lose_termination_means(&child);
        child.kill();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        assert_eq!(
            node.submit_clone(command.clone()).unwrap().state,
            ExecutionState::Unknown
        );
        assert!(record.target.path.is_dir());

        gate.release();
        let mut state = ExecutionState::Unknown;
        until(|| {
            node.recover_clones().unwrap();
            state = node.submit_clone(command.clone()).unwrap().state;
            state != ExecutionState::Unknown
        });
        assert_interrupted(&state, &record, &command);
        drop(node);
        assert_retained_attempts(&fixture, &record, &command, 2);
    });
}

/// A finished clone leaves nothing running: the Node's only child is its reaper, and no attempt
/// lock is left behind.
#[test]
fn completed_clone_leaves_no_process_but_the_reaper() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "resident", "main");
        seed(&fixture, &config, &command);
        let mut child = launch(&fixture, &config, "clone-resident");
        until(|| {
            fixture
                .log("clone-resident")
                .contains("Node recovery pass completed")
        });
        assert_eq!(
            (
                crate::support::descendants(child.0.id()),
                fs::read_dir(fixture.locks()).unwrap().count(),
            ),
            (vec![crate::support::reaper_program()], 0)
        );
        child.terminate();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        let state = node.submit_clone(command.clone()).unwrap().state;
        assert!(
            matches!(
                clone_result(&state),
                Some(CloneExecutionResult::CloneReady(_))
            ),
            "clone did not complete: {state:?}"
        );
    });
}

/// The seeded target, which an interrupted attempt must keep rather than delete or reuse.
fn retained(record: &CloneExecution) -> CloneResidual {
    CloneResidual::Retained {
        repository_id: record.target.repository_id.clone(),
        path: NodePath::new(record.target.path.to_str().unwrap()),
    }
}

/// Checks the interrupted failure; the reporting incarnation belongs to whichever process settled it.
fn assert_interrupted(
    state: &ExecutionState,
    record: &CloneExecution,
    command: &CloneRepositoryMessage,
) {
    let Some(CloneExecutionResult::CloneFailed(failed)) = clone_result(state) else {
        panic!("clone not settled as a failure: {state:?}");
    };
    assert_eq!(
        failed,
        &CloneFailed {
            node: failed.node.clone(),
            spec: command.payload.spec.clone(),
            failure: CloneFailureCode::Interrupted,
            residual: retained(record),
        }
    );
}

/// Every recorded attempt settled, and the original directory still in place.
fn assert_retained_attempts(
    fixture: &Fixture,
    record: &CloneExecution,
    command: &CloneRepositoryMessage,
    attempts: usize,
) {
    let database = NodeDatabase::open(
        &fixture.config().home_directory.join("ora-node.sqlite3"),
        fixture.config().identity,
    )
    .unwrap();
    let journal = database.process_journal().unwrap();
    assert_eq!(
        journal.attempts(&command.execution_id).unwrap().len(),
        attempts
    );
    assert!(journal.pending(&command.execution_id).unwrap().is_empty());
    assert!(record.target.path.is_dir());
}

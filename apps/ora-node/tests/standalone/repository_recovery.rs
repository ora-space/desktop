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
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "existing", "main");
        let record = seed(&fixture, &config, &command);
        fs::create_dir(&record.target.path).unwrap();
        let sentinel = record.target.path.join("user.txt");
        fs::write(&sentinel, "preserve").unwrap();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
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

/// Killing a real Node during HTTPS cannot authorize another Run or a directory-based success.
#[test]
fn killed_clone_recovers_original_run_without_redispatch() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let config = configuration(&fixture, &server);
        let command = request(&server, "kill", "main");
        let record = seed(&fixture, &config, &command);
        let mut child = launch(&fixture, &config, "clone-first");
        until(|| record.target.path.join(".git").is_dir());
        child.kill();
        server.paused.store(false, Ordering::SeqCst);
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
        node.recover_clones().unwrap();
        let state = node.submit_clone(command.clone()).unwrap().state;
        assert!(!matches!(
            state,
            ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneReady(_)))
        ));
        drop(node);
        let database = NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        let journal = database.process_journal().unwrap();
        assert_eq!(journal.attempts(&command.execution_id).unwrap().len(), 1);
        assert!(journal.pending(&command.execution_id).unwrap().is_empty());
        assert!(record.target.path.is_dir());
    });
}

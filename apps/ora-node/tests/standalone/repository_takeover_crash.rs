use super::*;
use crate::support::{ChildGuard, block_on, until};
use ora_controller::{
    CloneIntake, CoordinationStore, ExecutionOutcome, NodeEndpoint, NodeTarget, SessionConfig,
    SqliteStore, WriteGuard, WritePoint,
};
use pretty_assertions::assert_eq;
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct PauseBeforeCommit(PathBuf);
impl WriteGuard for PauseBeforeCommit {
    /// Signals the parent only after SQLite has staged the result, then waits for actual SIGKILL.
    fn before_write(&self, point: WritePoint) -> Result<(), ora_controller::Error> {
        if point == WritePoint::Commit {
            fs::write(&self.0, "transaction pending")?;
            loop {
                std::thread::park();
            }
        }
        Ok(())
    }
}

/// Child-only fixture runs the production session and SQLite owner with an injected transaction barrier.
#[test]
#[ignore = "subprocess fixture, invoked explicitly by the crash acceptance test"]
fn controller_transaction_child() {
    ora_logging::with_trace_logging(|| {
        let root = PathBuf::from(std::env::var_os("ORA_TEST_CONTROLLER_CRASH_ROOT").unwrap());
        let endpoint: NodeTarget =
            serde_json::from_slice(&fs::read(root.join("target.json")).unwrap()).unwrap();
        let store = SqliteStore::open_with_guard(
            &root.join("controller"),
            ControllerId::new("owner"),
            PauseBeforeCommit(root.join("before-commit")),
        )
        .unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                loop {
                    let _ = ora_controller::run_session(
                        &store,
                        &endpoint,
                        &SessionConfig {
                            io_timeout_ms: 5000,
                            query_interval_ms: 100,
                        },
                    )
                    .await;
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
            });
    });
}

/// Killing a real transaction cannot Ack or persist a partial takeover; reconnect takes over the original event.
#[test]
fn killed_controller_before_commit_replays_without_losing_node_responsibility() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let clone = configuration(&fixture, &server);
        let home = fixture.path().join("controller");
        let owner = SqliteStore::open(&home, ControllerId::new("owner")).unwrap();
        let command = block_on(owner.accept_request(
            RequestId::new("crash-request"),
            request(&server, "crash", "main").payload.spec,
        ))
        .unwrap();
        drop(owner);
        let mut node = ipc::launch(&fixture, &clone);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let proxy = controller::Proxy::new(&fixture);
        fs::write(
            fixture.path().join("target.json"),
            serde_json::to_vec(&NodeTarget {
                node_id: NodeId::new("test-node"),
                endpoint: NodeEndpoint::Ipc {
                    path: proxy.endpoint.clone(),
                },
            })
            .unwrap(),
        )
        .unwrap();
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "repository::takeover_crash::controller_transaction_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("ORA_TEST_CONTROLLER_CRASH_ROOT", fixture.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(/*secs*/ 40);
        while !fixture.path().join("before-commit").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "transaction child exited early"
            );
            assert!(
                Instant::now() < deadline,
                "takeover never reached pre-commit barrier"
            );
            std::thread::sleep(Duration::from_millis(/*millis*/ 10));
        }
        assert!(matches!(
            proxy.acknowledgements.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        child.kill();
        let owner = SqliteStore::open(&home, ControllerId::new("owner")).unwrap();
        assert_eq!(block_on(owner.result(&command.execution_id)).unwrap(), None);
        assert_eq!(
            block_on(owner.pending_dispatches(&NodeId::new("test-node"))).unwrap(),
            vec![command.clone()]
        );
        drop(owner);
        server.reject_auth.store(true, Ordering::SeqCst);
        proxy.drop_ack.store(false, Ordering::SeqCst);
        let mut replacement = controller::launch(&fixture, &proxy);
        let ack = proxy
            .acknowledgements
            .recv_timeout(Duration::from_secs(/*secs*/ 15))
            .unwrap();
        assert_eq!(ack.execution_id, command.execution_id);
        replacement.terminate();
        let owner = SqliteStore::open(&home, ControllerId::new("owner")).unwrap();
        assert!(matches!(
            block_on(owner.result(&command.execution_id)).unwrap(),
            Some(ExecutionOutcome::Ready { .. })
        ));
        node.terminate();
        let node = Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        let query = GetExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: command.operation_id,
            execution_id: command.execution_id,
            payload: GetExecutionStatus {
                node_id: NodeId::new("test-node"),
            },
        };
        let status = node.status(&query).unwrap();
        assert_eq!(
            status.payload.state,
            ExecutionState::Completed(ExecutionResult::Clone(
                block_on(owner.operation(&query.execution_id))
                    .unwrap()
                    .unwrap()
                    .result
                    .unwrap()
            ))
        );
    });
}

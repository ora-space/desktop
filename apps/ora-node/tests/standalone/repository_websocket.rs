use super::*;
use crate::support::{ChildGuard, until};
use ora_controller::{
    CloneIntake, CoordinationStore, ExecutionOutcome, NodeEndpoint, NodeTarget, SessionConfig,
    SessionError, SqliteStore,
};
use ora_node_transport::{
    CloseReason, FrameReceiver, FrameSender, TransportError,
    websocket::{self, ClientReceiver, ClientSender, WsEndpoint},
};
use pretty_assertions::assert_eq;
use rusqlite::OptionalExtension;
use std::{
    net::{Ipv4Addr, SocketAddr},
    process::{Command, Stdio},
    time::Duration,
};

pub(super) const PATH: &str = "/ora-node/v1";

/// Opens a raw Controller-side WebSocket and completes the owner's handshake, retrying while the
/// Node still holds admission for a peer that has just disconnected.
struct SessionSender {
    inner: ClientSender,
    binding: RuntimeBinding,
}

async fn hello(endpoint: &WsEndpoint) -> (ClientReceiver, SessionSender) {
    loop {
        let (mut receiver, mut sender) = websocket::connect(endpoint).await.unwrap();
        sender
            .send(
                encode_controller_frame(&ControllerToNodeMessage::Hello(HelloMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    payload: Hello {
                        controller_id: ControllerId::new("owner"),
                        supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                    },
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        match receiver.recv().await {
            Ok(Some(frame)) => {
                let NodeToControllerMessage::HelloAccepted(hello) =
                    decode_node_frame(&frame).unwrap()
                else {
                    panic!("expected authenticated greeting")
                };
                let root = endpoint.tls.as_ref().unwrap().ca_file.parent().unwrap();
                return (
                    receiver,
                    SessionSender {
                        inner: sender,
                        binding: runtime_fixture::binding(root, &hello.payload.node),
                    },
                );
            }
            Err(error) if error.is_busy() => {
                tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
            }
            other => panic!("unexpected handshake result {other:?}"),
        }
    }
}

/// Sends one Controller message as one binary WebSocket frame.
async fn send(sender: &mut SessionSender, message: &ControllerToNodeMessage) {
    let message = if let ControllerToNodeMessage::CloneRepository(command) = message {
        sender
            .inner
            .send(
                encode_controller_frame(&ControllerToNodeMessage::BindRuntime(
                    sender.binding.clone(),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        runtime_fixture::controlled(sender.binding.clone(), command.clone())
    } else {
        message.clone()
    };
    sender
        .inner
        .send(encode_controller_frame(&message).unwrap())
        .await
        .unwrap();
}

/// Reads the next Node message, bounded so a broken replay cannot hang the test.
async fn next(receiver: &mut ClientReceiver) -> NodeToControllerMessage {
    let frame = tokio::time::timeout(Duration::from_secs(/*secs*/ 40), receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    decode_node_frame(&frame).unwrap()
}

/// Waits for the next clone result event, skipping heartbeats and status replies.
async fn clone_result(receiver: &mut ClientReceiver) -> CloneResultMessage {
    loop {
        if let NodeToControllerMessage::CloneResult(event) = next(receiver).await {
            return event;
        }
    }
}

/// Observes the exact durable event without acquiring the live Node's exclusive writer lease.
fn pending_clone_event(fixture: &Fixture, execution: &ExecutionId) -> Option<CloneResultMessage> {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.config().home_directory.join("ora-node.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let event: Option<String> = connection
        .query_row(
            "SELECT event FROM clone_outbox WHERE execution=?1",
            [execution.as_str()],
            |row| row.get(/*idx*/ 0),
        )
        .optional()
        .unwrap();
    event.map(|event| {
        let NodeToControllerMessage::CloneResult(result) = serde_json::from_str(&event).unwrap()
        else {
            panic!("clone outbox contains a different event family");
        };
        result
    })
}

/// Starts a loopback-only library fixture for the original transport and crash obligations.
/// Production network authorization is exercised separately by mTLS and cluster acceptance.
pub(super) fn launch(fixture: &Fixture, clone: &CloneConfig, bind: SocketAddr) -> ChildGuard {
    runtime_fixture::advance_epoch(fixture.path());
    let config = fixture.path().join("websocket-config.json");
    fs::write(
        &config,
        serde_json::to_vec(&ora_node::ServiceConfig {
            plugins: Default::default(),
            agent: None,
            node: fixture.config(),
            process: fixture.process(),
            clone: Some(clone.clone()),
            control: Some(ora_node::ControlConfig {
                target: Some(runtime_fixture::scope()),
                controller_id: ControllerId::new("owner"),
                listen: ora_node::ControlListen::MutualTlsWebSocket {
                    bind,
                    path: PATH.into(),
                    tls: runtime_fixture::tls(fixture.path(), "node"),
                },
                heartbeat_ms: 100,
                frame_timeout_ms: 40_000,
            }),
            recovery_interval_ms: 50,
            timezone: "Asia/Shanghai".into(),
        })
        .unwrap(),
    )
    .unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_ora-node"))
        .arg(&config)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("separate workload OS identity"));
    ChildGuard(
        Command::new(
            std::path::Path::new(env!("CARGO_BIN_EXE_ora-node"))
                .with_file_name("examples")
                .join("loopback-transport-fixture"),
        )
        .arg(config)
        .stdin(Stdio::null())
        .stdout(fs::File::create(fixture.path().join("websocket.log")).unwrap())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap(),
    )
}

/// The production Controller session reaches a real Node over WebSocket with the same handshake,
/// takeover and admission rules as IPC: a clone result is durably taken over, a second connection
/// is refused with the recognizable busy close, and a Node with another identity is not dispatched to.
#[test]
fn controller_session_over_websocket_takes_over_clone_and_keeps_single_session() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let clone = configuration(&fixture, &server);
        // Reserve a free port; the Node binds it again after this listener closes.
        let bind = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let _node = launch(&fixture, &clone, bind);
        until(|| {
            fs::read_to_string(fixture.path().join("websocket.log"))
                .unwrap_or_default()
                .contains("Node mutual TLS WebSocket listening")
        });
        let store = SqliteStore::open(
            &fixture.path().join("controller"),
            ControllerId::new("owner"),
        )
        .unwrap();
        let store = runtime_fixture::RuntimeStore::new(store, fixture.path());
        let endpoint = runtime_fixture::endpoint(&fixture, bind);
        let target = NodeTarget {
            node_id: NodeId::new("test-node"),
            endpoint: NodeEndpoint::WebSocket(endpoint.clone()),
        };
        let settings = SessionConfig {
            io_timeout_ms: 40_000,
            query_interval_ms: 100,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = runtime.block_on(async {
            let command = store
                .accept_request(
                    RequestId::new("websocket"),
                    CloneExecutionSpec {
                        node_id: NodeId::new("test-node"),
                        repository: CloneRepositoryUrl::parse(&server.address).unwrap(),
                        branch: BranchName::new("main"),
                    },
                )
                .await
                .unwrap();
            let session = ora_controller::run_session(&store, &target, &settings);
            let observe = async {
                let outcome = loop {
                    if let Some(outcome) = store.result(&command.execution_id).await.unwrap() {
                        break outcome;
                    }
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                };
                // The live session still owns admission, so another peer is refused end to end.
                let (mut receiver, _sender) = websocket::connect(&endpoint).await.unwrap();
                assert!(receiver.recv().await.unwrap_err().is_busy());
                outcome
            };
            let outcome = tokio::select! {
                result = session => panic!("session ended before takeover: {result:?}"),
                outcome = observe => outcome,
            };
            // Dropping the session releases admission once the Node sees the connection end.
            let wrong = NodeTarget {
                node_id: NodeId::new("other-node"),
                endpoint: NodeEndpoint::WebSocket(endpoint.clone()),
            };
            let refused = loop {
                match ora_controller::run_session(&store, &wrong, &settings).await {
                    Err(SessionError::Busy) => {
                        tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                    }
                    other => break other,
                }
            };
            assert!(
                matches!(refused, Err(SessionError::Mismatch(_))),
                "{refused:?}"
            );
            outcome
        });
        assert!(
            matches!(outcome, ExecutionOutcome::Ready { .. }),
            "{outcome:?}"
        );
    });
}

/// Over WebSocket, an unacknowledged clone result survives disconnection and Node restart and is
/// replayed with its original identity; the production Controller session then takes over the
/// replayed event and acknowledges it, after which the Node stops replaying it.
#[test]
fn websocket_replays_unacknowledged_result_across_disconnect_and_restart_until_controller_acks() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let clone = configuration(&fixture, &server);
        let bind = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        // Each launch recreates the log, so readiness is the first listening line of that run.
        let listening = |fixture: &Fixture| {
            fs::read_to_string(fixture.path().join("websocket.log"))
                .unwrap_or_default()
                .contains("Node mutual TLS WebSocket listening")
        };
        let mut node = launch(&fixture, &clone, bind);
        until(|| listening(&fixture));
        let store = SqliteStore::open(
            &fixture.path().join("controller"),
            ControllerId::new("owner"),
        )
        .unwrap();
        let store = runtime_fixture::RuntimeStore::new(store, fixture.path());
        let endpoint = runtime_fixture::endpoint(&fixture, bind);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (command, original) = runtime.block_on(async {
            let command = store
                .accept_request(
                    RequestId::new("websocket-replay"),
                    CloneExecutionSpec {
                        node_id: NodeId::new("test-node"),
                        repository: CloneRepositoryUrl::parse(&server.address).unwrap(),
                        branch: BranchName::new("main"),
                    },
                )
                .await
                .unwrap();
            let (mut receiver, mut sender) = hello(&endpoint).await;
            send(
                &mut sender,
                &ControllerToNodeMessage::CloneRepository(command.clone()),
            )
            .await;
            let original = clone_result(&mut receiver).await;
            let CloneExecutionResult::CloneReady(ready) = &original.payload else {
                panic!("fixture clone did not complete");
            };
            // Model the actual commit-before-ACK loss window: Controller takeover is durable,
            // but no ACK reaches Node. Its exact event must still survive disconnect/restart.
            store
                .take_over_node_event(&ready.node, &original)
                .await
                .unwrap();
            assert!(matches!(
                store.result(&command.execution_id).await.unwrap(),
                Some(ExecutionOutcome::Ready { .. })
            ));
            assert_eq!(
                pending_clone_event(&fixture, &command.execution_id),
                Some(original.clone())
            );
            drop((receiver, sender));
            let (mut receiver, _sender) = hello(&endpoint).await;
            assert_eq!(clone_result(&mut receiver).await, original);
            (command, original)
        });
        // Node restart changes the incarnation but not the retained event.
        node.kill();
        let mut node = launch(&fixture, &clone, bind);
        until(|| listening(&fixture));
        runtime.block_on(async {
            let (mut receiver, _sender) = hello(&endpoint).await;
            assert_eq!(clone_result(&mut receiver).await, original);
        });
        runtime.block_on(async {
            let target = NodeTarget {
                node_id: NodeId::new("test-node"),
                endpoint: NodeEndpoint::WebSocket(endpoint.clone()),
            };
            let settings = SessionConfig {
                io_timeout_ms: 40_000,
                query_interval_ms: 100,
            };
            // The first connection attempt may race the previous peer's release of admission.
            let session = async {
                loop {
                    let _ = ora_controller::run_session(&store, &target, &settings).await;
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
            };
            let taken_over = async {
                loop {
                    // Controller commit precedes its wire ACK. Keep the real session alive until
                    // Node has durably consumed the exact event; cancelling at result visibility
                    // reproduces the legitimate commit-before-ACK loss window above.
                    if let Some(outcome) = store.result(&command.execution_id).await.unwrap()
                        && pending_clone_event(&fixture, &command.execution_id).is_none()
                    {
                        break outcome;
                    }
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
            };
            let outcome =
                tokio::time::timeout(Duration::from_millis(settings.io_timeout_ms), async {
                    tokio::select! {
                        () = session => unreachable!(),
                        outcome = taken_over => outcome,
                    }
                })
                .await
                .expect("Controller takeover never reached Node's durable ACK");
            assert!(
                matches!(outcome, ExecutionOutcome::Ready { .. }),
                "{outcome:?}"
            );
        });
        // Once acknowledged, the event is no longer replayed: only heartbeats arrive while a status
        // query still reports the retained terminal result.
        runtime.block_on(async {
            let (mut receiver, mut sender) = hello(&endpoint).await;
            send(
                &mut sender,
                &ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id.clone(),
                    execution_id: command.execution_id.clone(),
                    payload: GetExecutionStatus {
                        node_id: NodeId::new("test-node"),
                    },
                }),
            )
            .await;
            let deadline =
                tokio::time::Instant::now() + Duration::from_millis(/*millis*/ 1000);
            let mut completed = false;
            while tokio::time::Instant::now() < deadline {
                let Ok(frame) = tokio::time::timeout_at(deadline, receiver.recv()).await else {
                    break;
                };
                match decode_node_frame(&frame.unwrap().unwrap()).unwrap() {
                    NodeToControllerMessage::Heartbeat(_) => {}
                    NodeToControllerMessage::ExecutionStatus(status) => {
                        assert!(matches!(status.payload.state, ExecutionState::Completed(_)));
                        completed = true;
                    }
                    message => panic!("acknowledged event must not be replayed: {message:?}"),
                }
            }
            assert!(completed);
        });
        node.kill();
    });
}

/// A clone cut short by a normal Node stop reaches the Controller as a terminal interrupted failure
/// over WebSocket: the stopping Node settles it, the restarted Node replays it, the production
/// session takes it over, and the execution leaves the Controller's pending queries for good.
#[test]
fn controller_session_takes_over_interrupted_clone_after_node_stop() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let clone = configuration(&fixture, &server);
        let bind = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let listening = |fixture: &Fixture| {
            fs::read_to_string(fixture.path().join("websocket.log"))
                .unwrap_or_default()
                .contains("Node mutual TLS WebSocket listening")
        };
        let mut node = launch(&fixture, &clone, bind);
        until(|| listening(&fixture));
        let store = SqliteStore::open(
            &fixture.path().join("controller"),
            ControllerId::new("owner"),
        )
        .unwrap();
        let store = runtime_fixture::RuntimeStore::new(store, fixture.path());
        let target = NodeTarget {
            node_id: NodeId::new("test-node"),
            endpoint: NodeEndpoint::WebSocket(runtime_fixture::endpoint(&fixture, bind)),
        };
        let settings = SessionConfig {
            io_timeout_ms: 40_000,
            query_interval_ms: 100,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Retries cover the window in which the Node still holds admission for a dropped peer.
        let session = || async {
            loop {
                let _ = ora_controller::run_session(&store, &target, &settings).await;
                tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
            }
        };
        let (command, directory) = runtime.block_on(async {
            let command = store
                .accept_request(
                    RequestId::new("websocket-interrupted"),
                    CloneExecutionSpec {
                        node_id: NodeId::new("test-node"),
                        repository: CloneRepositoryUrl::parse(&server.address).unwrap(),
                        branch: BranchName::new("main"),
                    },
                )
                .await
                .unwrap();
            // The session dispatches the clone; Git then blocks on the paused HTTPS server.
            let dispatched = async {
                loop {
                    let started = fs::read_dir(&clone.repository_root)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .find(|path| path.join(".git").is_dir());
                    if let Some(directory) = started {
                        break directory;
                    }
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
            };
            let directory = tokio::select! {
                () = session() => unreachable!(),
                directory = dispatched => directory,
            };
            (command, directory)
        });
        node.terminate();
        let mut node = launch(&fixture, &clone, bind);
        until(|| listening(&fixture));
        let outcome = runtime.block_on(async {
            let taken_over = async {
                loop {
                    if let Some(outcome) = store.result(&command.execution_id).await.unwrap() {
                        break outcome;
                    }
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
            };
            tokio::select! {
                () = session() => unreachable!(),
                outcome = taken_over => outcome,
            }
        });
        let ExecutionOutcome::Failed { node: reporter, .. } = &outcome else {
            panic!("interrupted clone must fail: {outcome:?}");
        };
        assert_eq!(
            outcome,
            ExecutionOutcome::Failed {
                node: reporter.clone(),
                failure: CloneFailureCode::Interrupted,
                retained_path: Some(NodePath::new(directory.to_str().unwrap())),
            }
        );
        // A terminal result is no longer queried, unlike the permanent Unknown it replaces.
        assert_eq!(
            runtime
                .block_on(store.pending_dispatches(&NodeId::new("test-node")))
                .unwrap(),
            vec![]
        );
        assert!(directory.is_dir());
        node.kill();
    });
}

/// The Node ends sessions with a close code naming why: a Controller that does not own it gets
/// the identity code (at Hello or through a foreign heartbeat), malformed input the protocol code,
/// and a normal stop the shutdown code,
/// so a router forwards the reason instead of reporting a lost connection.
#[test]
fn node_closes_sessions_with_the_reason_code() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let clone = configuration(&fixture, &server);
        let bind = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let mut node = launch(&fixture, &clone, bind);
        until(|| {
            fs::read_to_string(fixture.path().join("websocket.log"))
                .unwrap_or_default()
                .contains("Node mutual TLS WebSocket listening")
        });
        let endpoint = runtime_fixture::endpoint(&fixture, bind);
        let closed = |result: Result<Option<Vec<u8>>, TransportError>| result.unwrap_err();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (mut receiver, mut sender) = websocket::connect(&endpoint).await.unwrap();
            sender
                .send(
                    encode_controller_frame(&ControllerToNodeMessage::Hello(HelloMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        payload: Hello {
                            controller_id: ControllerId::new("intruder"),
                            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                        },
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
            let refused = closed(receiver.recv().await);
            assert!(
                refused.is_closed_for(CloseReason::IdentityMismatch),
                "{refused:?}"
            );

            let (mut receiver, mut sender) = hello(&endpoint).await;
            sender.inner.send(vec![0xff, b'{']).await.unwrap();
            let violation = loop {
                match receiver.recv().await {
                    // Heartbeats sent before the Node read the bad frame.
                    Ok(Some(_)) => {}
                    other => break closed(other),
                }
            };
            assert!(
                violation.is_closed_for(CloseReason::ProtocolViolation),
                "{violation:?}"
            );

            let (mut receiver, mut sender) = hello(&endpoint).await;
            send(
                &mut sender,
                &ControllerToNodeMessage::Heartbeat(ControllerHeartbeatMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    payload: ControllerHeartbeat {
                        controller_id: ControllerId::new("intruder"),
                    },
                }),
            )
            .await;
            let foreign = loop {
                match receiver.recv().await {
                    Ok(Some(_)) => {}
                    other => break closed(other),
                }
            };
            assert!(
                foreign.is_closed_for(CloseReason::IdentityMismatch),
                "{foreign:?}"
            );

            let (mut receiver, _sender) = hello(&endpoint).await;
            // Signal without waiting: the Node's close waits for this side to answer it.
            let status = Command::new("kill")
                .arg("-TERM")
                .arg(node.0.id().to_string())
                .status()
                .unwrap();
            assert!(status.success());
            let stopping = loop {
                match receiver.recv().await {
                    Ok(Some(_)) => {}
                    other => break closed(other),
                }
            };
            assert!(
                stopping.is_closed_for(CloseReason::Shutdown),
                "{stopping:?}"
            );
        });
        node.terminate();
    });
}

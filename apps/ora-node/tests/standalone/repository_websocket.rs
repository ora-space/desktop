use super::*;
use crate::support::{ChildGuard, until};
use ora_controller::{
    CloneIntake, CoordinationStore, ExecutionOutcome, NodeEndpoint, NodeTarget, SessionConfig,
    SessionError, SqliteStore,
};
use ora_node_transport::{
    FrameReceiver, FrameSender,
    websocket::{self, ClientReceiver, ClientSender, WsEndpoint},
};
use pretty_assertions::assert_eq;
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    process::{Command, Stdio},
    time::Duration,
};

const PATH: &str = "/ora-node/v1";

/// Opens a raw Controller-side WebSocket and completes the owner's handshake, retrying while the
/// Node still holds admission for a peer that has just disconnected.
async fn hello(endpoint: &WsEndpoint) -> (ClientReceiver, ClientSender) {
    loop {
        let (mut receiver, mut sender) = websocket::connect(endpoint).await.unwrap();
        send(
            &mut sender,
            &ControllerToNodeMessage::Hello(HelloMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                payload: Hello {
                    controller_id: ControllerId::new("owner"),
                    supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                },
            }),
        )
        .await;
        match receiver.recv().await {
            Ok(Some(frame)) => {
                assert!(matches!(
                    decode_node_frame(&frame).unwrap(),
                    NodeToControllerMessage::HelloAccepted(_)
                ));
                return (receiver, sender);
            }
            Err(error) if error.is_busy() => {
                tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
            }
            other => panic!("unexpected handshake result {other:?}"),
        }
    }
}

/// Sends one Controller message as one binary WebSocket frame.
async fn send(sender: &mut ClientSender, message: &ControllerToNodeMessage) {
    sender
        .send(encode_controller_frame(message).unwrap())
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

/// Starts the production service listening for WebSocket upgrades instead of a local socket.
fn launch(fixture: &Fixture, clone: &CloneConfig, bind: SocketAddr) -> ChildGuard {
    let config = fixture.path().join("websocket-config.json");
    fs::write(
        &config,
        serde_json::to_vec(&ora_node::ServiceConfig {
            node: fixture.config(),
            process: fixture.process(),
            clone: Some(clone.clone()),
            control: Some(ora_node::ControlConfig {
                controller_id: ControllerId::new("owner"),
                listen: ora_node::ControlListen::WebSocket {
                    bind,
                    path: PATH.into(),
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
    ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_ora-node"))
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
                .contains("Node WebSocket listening")
        });
        let store = SqliteStore::open(
            &fixture.path().join("controller"),
            ControllerId::new("owner"),
        )
        .unwrap();
        let endpoint = WsEndpoint {
            url: format!("ws://{bind}{PATH}"),
            headers: BTreeMap::new(),
        };
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
                matches!(refused, Err(SessionError::Protocol(_))),
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
                .contains("Node WebSocket listening")
        };
        let mut node = launch(&fixture, &clone, bind);
        until(|| listening(&fixture));
        let store = SqliteStore::open(
            &fixture.path().join("controller"),
            ControllerId::new("owner"),
        )
        .unwrap();
        let endpoint = WsEndpoint {
            url: format!("ws://{bind}{PATH}"),
            headers: BTreeMap::new(),
        };
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
            // The acknowledgement is lost with the connection: reconnecting replays the same event.
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
                    if let Some(outcome) = store.result(&command.execution_id).await.unwrap() {
                        break outcome;
                    }
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
            };
            let outcome = tokio::select! {
                () = session => unreachable!(),
                outcome = taken_over => outcome,
            };
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

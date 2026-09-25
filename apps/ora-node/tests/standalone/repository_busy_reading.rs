//! The session keeps reading while Git occupies the worker, so a peer that leaves releases the
//! control slot at once instead of after the admission deadline, and router pings are answered.
use super::*;
use crate::support::until;
use futures_util::{SinkExt, StreamExt};
use ora_node_transport::{
    FrameReceiver, FrameSender,
    websocket::{self as transport, WsEndpoint},
};
use pretty_assertions::assert_eq;
use std::{collections::BTreeMap, net::Ipv4Addr, time::Duration};
use tokio::{io::AsyncWriteExt, net::UnixStream, time::timeout};
use tokio_tungstenite::tungstenite::{
    Message,
    protocol::{CloseFrame, frame::coding::CloseCode},
};

/// Far below the 40-second admission deadline both launchers configure, so passing within it
/// proves the Node noticed the departed peer rather than timing out the unanswered request.
const RELEASE_BOUND: Duration = Duration::from_secs(/*secs*/ 5);

/// How long a status query must stay unanswered before the worker counts as occupied by Git.
const UNANSWERED: Duration = Duration::from_millis(/*millis*/ 800);

/// The deployment owner's handshake.
fn hello() -> ControllerToNodeMessage {
    ControllerToNodeMessage::Hello(HelloMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("owner"),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    })
}

/// Asks for the status of `command`; while Git occupies the worker the reply stays queued.
fn status_query(command: &CloneRepositoryMessage) -> ControllerToNodeMessage {
    ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        payload: GetExecutionStatus {
            node_id: NodeId::new("test-node"),
        },
    })
}

/// Reads until the Accepted status, proving the clone is durable and will occupy the worker.
async fn accepted_ipc(stream: &mut UnixStream) {
    loop {
        if let Some(NodeToControllerMessage::ExecutionStatus(status)) =
            read_node_message(stream).await.unwrap()
        {
            assert_eq!(status.payload.state, ExecutionState::Accepted);
            return;
        }
    }
}

/// Over IPC, ending a session whose status query waits behind Git frees the slot immediately.
#[test]
fn ipc_peer_leaving_while_worker_runs_git_releases_the_control_slot() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child = ipc::launch(&fixture, &config);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let command = request(&server, "busy-ipc", "main");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut stream = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    read_node_message(&mut stream).await.unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                write_controller_message(
                    &mut stream,
                    &ControllerToNodeMessage::CloneRepository(command.clone()),
                )
                .await
                .unwrap();
                accepted_ipc(&mut stream).await;
                // The worker picks the clone up on its next recovery pass; until then a query is
                // answered. A query left unanswered for a while is waiting behind the paused Git.
                loop {
                    write_controller_message(&mut stream, &status_query(&command))
                        .await
                        .unwrap();
                    let answered = timeout(UNANSWERED, async {
                        loop {
                            match read_node_message(&mut stream).await.unwrap() {
                                Some(NodeToControllerMessage::Heartbeat(_)) => {}
                                Some(NodeToControllerMessage::ExecutionStatus(_)) => return,
                                other => panic!("unexpected message {other:?}"),
                            }
                        }
                    })
                    .await;
                    if answered.is_err() {
                        break;
                    }
                }
                // Half-close like a peer that has finished but whose socket lingers: heartbeat
                // writes still succeed, so only reading the end of stream can release the slot.
                stream.shutdown().await.unwrap();
                timeout(RELEASE_BOUND, async {
                    loop {
                        // A refused IPC peer sees the socket close during the handshake, which
                        // can surface as a failed Hello write as well as an empty read.
                        let mut replacement = UnixStream::connect(&endpoint).await.unwrap();
                        let written = write_controller_message(&mut replacement, &hello()).await;
                        let reply = match written {
                            Ok(()) => read_node_message(&mut replacement).await,
                            Err(error) => Err(error),
                        };
                        match reply {
                            Ok(Some(NodeToControllerMessage::HelloAccepted(_))) => return,
                            Ok(None) | Err(_) => {
                                tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                            }
                            Ok(Some(other)) => panic!("unexpected handshake reply {other:?}"),
                        }
                    }
                })
                .await
                .expect("a departed peer must not hold the control slot while Git runs");
            });
        server.paused.store(false, Ordering::SeqCst);
        child.terminate();
    });
}

/// Over WebSocket, the Node answers pings and honors a close while its worker is busy, so a peer
/// reconnecting through a restarted router is admitted instead of refused with `4409`.
#[test]
fn websocket_session_answers_ping_and_close_while_worker_runs_git() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let clone = configuration(&fixture, &server);
        // Reserve a free port; the Node binds it again after this listener closes.
        let bind = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let _node = websocket::launch(&fixture, &clone, bind);
        until(|| {
            fs::read_to_string(fixture.path().join("websocket.log"))
                .unwrap_or_default()
                .contains("Node WebSocket listening")
        });
        let url = format!("ws://{bind}{}", websocket::PATH);
        let command = request(&server, "busy-websocket", "main");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
                    .await
                    .unwrap();
                for message in [
                    hello(),
                    ControllerToNodeMessage::CloneRepository(command.clone()),
                ] {
                    let frame = encode_controller_frame(&message).unwrap();
                    socket.send(Message::Binary(frame.into())).await.unwrap();
                }
                let mut accepted = false;
                while !accepted {
                    let Message::Binary(frame) = socket.next().await.unwrap().unwrap() else {
                        continue;
                    };
                    accepted = matches!(
                        decode_node_frame(&frame).unwrap(),
                        NodeToControllerMessage::ExecutionStatus(status)
                            if status.payload.state == ExecutionState::Accepted
                    );
                }
                // As over IPC, keep querying until one query stays unanswered behind the paused Git.
                loop {
                    let query = encode_controller_frame(&status_query(&command)).unwrap();
                    socket.send(Message::Binary(query.into())).await.unwrap();
                    let answered = timeout(UNANSWERED, async {
                        loop {
                            let Message::Binary(frame) = socket.next().await.unwrap().unwrap()
                            else {
                                panic!("unexpected control message before the probe");
                            };
                            match decode_node_frame(&frame).unwrap() {
                                NodeToControllerMessage::Heartbeat(_) => {}
                                NodeToControllerMessage::ExecutionStatus(_) => return,
                                other => panic!("unexpected message {other:?}"),
                            }
                        }
                    })
                    .await;
                    if answered.is_err() {
                        break;
                    }
                }
                socket
                    .send(Message::Ping(b"router".to_vec().into()))
                    .await
                    .unwrap();
                timeout(RELEASE_BOUND, async {
                    loop {
                        match socket.next().await.unwrap().unwrap() {
                            Message::Pong(payload) => {
                                assert_eq!(payload.as_ref(), b"router");
                                return;
                            }
                            Message::Binary(frame) => assert!(matches!(
                                decode_node_frame(&frame).unwrap(),
                                NodeToControllerMessage::Heartbeat(_)
                            )),
                            other => panic!("unexpected message {other:?}"),
                        }
                    }
                })
                .await
                .expect("a busy worker must not stop the Node from answering pings");
                socket
                    .send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "router restarting".into(),
                    })))
                    .await
                    .unwrap();
                // A router waiting for the close handshake keeps the TCP connection open, so
                // heartbeat writes still succeed and only reading the close can release the slot.
                let _lingering = socket;
                let endpoint = WsEndpoint {
                    url,
                    headers: BTreeMap::new(),
                };
                timeout(RELEASE_BOUND, async {
                    loop {
                        let (mut receiver, mut sender) =
                            transport::connect(&endpoint).await.unwrap();
                        sender
                            .send(encode_controller_frame(&hello()).unwrap())
                            .await
                            .unwrap();
                        match receiver.recv().await {
                            Ok(Some(frame)) => {
                                assert!(matches!(
                                    decode_node_frame(&frame).unwrap(),
                                    NodeToControllerMessage::HelloAccepted(_)
                                ));
                                return;
                            }
                            // Only the instant between our close and the Node reading it may
                            // still see the slot occupied.
                            Err(error) if error.is_busy() => {
                                tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                            }
                            other => panic!("unexpected handshake result {other:?}"),
                        }
                    }
                })
                .await
                .expect("a closed peer must not hold the control slot while Git runs");
            });
        server.paused.store(false, Ordering::SeqCst);
    });
}

/// A Controller polling on its timer during a long Git pass outruns the unanswered-request bound;
/// the excess queries are shed rather than closing the session, and answers resume after Git.
#[test]
fn polling_beyond_the_unanswered_bound_keeps_the_session_while_git_runs() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child = ipc::launch(&fixture, &config);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let command = request(&server, "polled", "main");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut stream = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    read_node_message(&mut stream).await.unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                write_controller_message(
                    &mut stream,
                    &ControllerToNodeMessage::CloneRepository(command.clone()),
                )
                .await
                .unwrap();
                accepted_ipc(&mut stream).await;
                // Wait until a query stays unanswered, then send several times the bound more.
                loop {
                    write_controller_message(&mut stream, &status_query(&command))
                        .await
                        .unwrap();
                    let answered = timeout(UNANSWERED, async {
                        while !matches!(
                            read_node_message(&mut stream).await.unwrap(),
                            Some(NodeToControllerMessage::ExecutionStatus(_))
                        ) {}
                    })
                    .await;
                    if answered.is_err() {
                        break;
                    }
                }
                for _ in 0..48 {
                    write_controller_message(&mut stream, &status_query(&command))
                        .await
                        .unwrap();
                }
                // The session outlives the flood: heartbeats keep arriving while Git is paused.
                for _ in 0..5 {
                    assert!(matches!(
                        timeout(
                            Duration::from_secs(/*secs*/ 2),
                            read_node_message(&mut stream)
                        )
                        .await
                        .unwrap()
                        .unwrap(),
                        Some(NodeToControllerMessage::Heartbeat(_))
                    ));
                }
                server.paused.store(false, Ordering::SeqCst);
                timeout(Duration::from_secs(/*secs*/ 35), async {
                    loop {
                        match read_node_message(&mut stream).await.unwrap() {
                            Some(NodeToControllerMessage::ExecutionStatus(status)) => {
                                assert_eq!(status.execution_id, command.execution_id);
                                return;
                            }
                            Some(_) => {}
                            None => panic!("session closed after the flood of queries"),
                        }
                    }
                })
                .await
                .expect("queued queries are answered once Git finishes");
            });
        child.terminate();
    });
}

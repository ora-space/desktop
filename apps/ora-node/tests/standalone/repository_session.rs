use super::*;
use crate::support::until;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::time::timeout;

/// A non-reading peer cannot pin the session or erase an unacknowledged terminal result.
#[test]
fn slow_reader_is_disconnected_and_original_result_is_replayed() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child =
            ipc::launch_with_deadline(&fixture, &config, /*frame_timeout_ms*/ 3000);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let command = request(&server, "backpressure", "main");
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
                let event = timeout(Duration::from_secs(/*secs*/ 40), async {
                    loop {
                        match read_node_message(&mut stream).await.unwrap() {
                            Some(NodeToControllerMessage::CloneResult(event)) => break event,
                            None => {
                                stream = ipc::connect(&endpoint, "owner").await;
                                assert!(matches!(
                                    read_node_message(&mut stream).await.unwrap(),
                                    Some(NodeToControllerMessage::HelloAccepted(_))
                                ));
                            }
                            Some(_) => {}
                        }
                    }
                })
                .await
                .unwrap();
                let query =
                    ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: command.operation_id.clone(),
                        execution_id: command.execution_id.clone(),
                        payload: GetExecutionStatus {
                            node_id: NodeId::new("test-node"),
                        },
                    });
                // Deliberately stop consuming responses while sending enough queries to exceed socket
                // and application buffers. Keep both halves alive: peer EOF must not cause the close.
                let mut sent = 0;
                let _ = timeout(Duration::from_secs(/*secs*/ 8), async {
                    for _ in 0..20_000 {
                        if write_controller_message(&mut stream, &query).await.is_err() {
                            break;
                        }
                        sent += 1;
                    }
                })
                .await;
                assert!(
                    sent > 16,
                    "exercise queue pressure, not just an idle connection"
                );
                timeout(Duration::from_secs(/*secs*/ 6), async {
                    // A timed-out partial write may leave a truncated final frame.
                    while let Ok(Some(_)) = read_node_message(&mut stream).await {}
                })
                .await
                .expect("backpressure must release the session");
                let mut replacement = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    read_node_message(&mut replacement).await.unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                let replay = timeout(Duration::from_secs(/*secs*/ 2), async {
                    loop {
                        if let Some(NodeToControllerMessage::CloneResult(replay)) =
                            read_node_message(&mut replacement).await.unwrap()
                        {
                            break replay;
                        }
                    }
                })
                .await
                .unwrap();
                assert_eq!(replay, event);
            });
        child.terminate();
    });
}

/// Partial bodies cannot hold admission indefinitely, and the next connection gets a fresh frame boundary.
#[test]
fn partial_frame_expires_and_releases_the_control_session() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child =
            ipc::launch_with_deadline(&fixture, &config, /*frame_timeout_ms*/ 3000);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use tokio::io::AsyncWriteExt;
                let mut stream = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    read_node_message(&mut stream).await.unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                stream.write_all(&[0, 0, 0, 32, 1, b'{']).await.unwrap();
                timeout(Duration::from_secs(/*secs*/ 6), async {
                    while let Some(message) = read_node_message(&mut stream).await.unwrap() {
                        assert!(matches!(message, NodeToControllerMessage::Heartbeat(_)));
                    }
                })
                .await
                .expect("incomplete frame must expire");
                let mut replacement = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    timeout(
                        Duration::from_secs(/*secs*/ 2),
                        read_node_message(&mut replacement)
                    )
                    .await
                    .unwrap()
                    .unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
            });
        child.terminate();
    });
}

/// A queued command has a finite wait; revocation prevents later admission without canceling accepted Git.
#[test]
fn busy_clone_keeps_heartbeats_and_revokes_timed_out_queued_command() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child =
            ipc::launch_with_deadline(&fixture, &config, /*frame_timeout_ms*/ 3000);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let first = request(&server, "busy", "main");
        let queued = request(&server, "revoked", "main");
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
                    &ControllerToNodeMessage::CloneRepository(first.clone()),
                )
                .await
                .unwrap();
                // Receiving Accepted proves the first command is durable before testing disconnect semantics.
                loop {
                    if let Some(NodeToControllerMessage::ExecutionStatus(status)) =
                        read_node_message(&mut stream).await.unwrap()
                    {
                        assert_eq!(status.payload.state, ExecutionState::Accepted);
                        break;
                    }
                }
                let mut beats = 0;
                while beats < 5 {
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
                    beats += 1;
                }
                write_controller_message(
                    &mut stream,
                    &ControllerToNodeMessage::CloneRepository(queued.clone()),
                )
                .await
                .unwrap();
                timeout(Duration::from_secs(/*secs*/ 6), async {
                    while let Some(message) = read_node_message(&mut stream).await.unwrap() {
                        assert!(matches!(message, NodeToControllerMessage::Heartbeat(_)));
                    }
                })
                .await
                .expect("queued requests must not keep a session alive indefinitely");
                server.paused.store(false, Ordering::SeqCst);
                let mut replacement = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    read_node_message(&mut replacement).await.unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                write_controller_message(
                    &mut replacement,
                    &ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: queued.operation_id.clone(),
                        execution_id: queued.execution_id.clone(),
                        payload: GetExecutionStatus {
                            node_id: NodeId::new("test-node"),
                        },
                    }),
                )
                .await
                .unwrap();
                let mut completed = false;
                let mut unknown = false;
                timeout(Duration::from_secs(/*secs*/ 35), async {
                    while !completed || !unknown {
                        let Some(message) = read_node_message(&mut replacement).await.unwrap()
                        else {
                            // Recovery can outlast a new query deadline as well; reconnect without
                            // ever resubmitting either command, retaining the original execution.
                            replacement = ipc::connect(&endpoint, "owner").await;
                            assert!(matches!(
                                read_node_message(&mut replacement).await.unwrap(),
                                Some(NodeToControllerMessage::HelloAccepted(_))
                            ));
                            write_controller_message(
                                &mut replacement,
                                &ControllerToNodeMessage::GetExecutionStatus(
                                    GetExecutionStatusMessage {
                                        protocol_version: CURRENT_PROTOCOL_VERSION,
                                        operation_id: queued.operation_id.clone(),
                                        execution_id: queued.execution_id.clone(),
                                        payload: GetExecutionStatus {
                                            node_id: NodeId::new("test-node"),
                                        },
                                    },
                                ),
                            )
                            .await
                            .unwrap();
                            continue;
                        };
                        match message {
                            NodeToControllerMessage::CloneResult(event) => {
                                assert_eq!(event.execution_id, first.execution_id);
                                assert!(matches!(
                                    event.payload,
                                    CloneExecutionResult::CloneReady(_)
                                ));
                                completed = true;
                            }
                            NodeToControllerMessage::ExecutionStatus(status) => {
                                assert_eq!(status.execution_id, queued.execution_id);
                                assert_eq!(status.payload.state, ExecutionState::Unknown);
                                unknown = true;
                            }
                            NodeToControllerMessage::Heartbeat(_) => {}
                            message => panic!("unexpected {message:?}"),
                        }
                    }
                })
                .await
                .unwrap();
            });
        child.terminate();
    });
}

/// An idle Controller keeps its session past the read deadline with heartbeats alone, and a
/// heartbeat naming another Controller ends the session and releases admission.
#[test]
fn controller_heartbeats_keep_idle_session_and_foreign_heartbeat_ends_it() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child =
            ipc::launch_with_deadline(&fixture, &config, /*frame_timeout_ms*/ 1000);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let heartbeat = |owner: &str| {
            ControllerToNodeMessage::Heartbeat(ControllerHeartbeatMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                payload: ControllerHeartbeat {
                    controller_id: ControllerId::new(owner),
                },
            })
        };
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
                {
                    let (mut reader, mut writer) = stream.split();
                    let beat = async {
                        let mut tick = tokio::time::interval(Duration::from_millis(/*millis*/ 250));
                        loop {
                            tick.tick().await;
                            write_controller_message(&mut writer, &heartbeat("owner"))
                                .await
                                .unwrap();
                        }
                    };
                    let drain = async {
                        loop {
                            let message = read_node_message(&mut reader).await.unwrap();
                            assert!(
                                matches!(message, Some(NodeToControllerMessage::Heartbeat(_))),
                                "idle session with Controller heartbeats must stay open, got {message:?}"
                            );
                        }
                    };
                    // Three read deadlines with no command traffic: only heartbeats flow.
                    let idle = timeout(Duration::from_secs(/*secs*/ 3), async {
                        tokio::select! { () = beat => {}, () = drain => {} }
                    })
                    .await;
                    assert!(idle.is_err(), "idle session ended before the observation window");
                }
                // A refused IPC connection may be closed before or after our Hello is written, so
                // the write can fail and the read may see end of stream or a reset; each means
                // admission was not granted.
                let mut rejected = tokio::net::UnixStream::connect(&endpoint).await.unwrap();
                let hello = ControllerToNodeMessage::Hello(HelloMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    payload: Hello {
                        controller_id: ControllerId::new("owner"),
                        supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                    },
                });
                let _ = write_controller_message(&mut rejected, &hello).await;
                let refusal =
                    timeout(Duration::from_secs(/*secs*/ 2), read_node_message(&mut rejected))
                        .await
                        .unwrap();
                assert!(
                    matches!(refusal, Ok(None) | Err(FrameError::Io(_))),
                    "the heartbeating session must still own admission, got {refusal:?}"
                );
                write_controller_message(&mut stream, &heartbeat("other"))
                    .await
                    .unwrap();
                timeout(Duration::from_secs(/*secs*/ 2), async {
                    while let Some(message) = read_node_message(&mut stream).await.unwrap() {
                        assert!(matches!(message, NodeToControllerMessage::Heartbeat(_)));
                    }
                })
                .await
                .expect("a foreign Controller heartbeat must end the session");
                let mut replacement = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    timeout(
                        Duration::from_secs(/*secs*/ 2),
                        read_node_message(&mut replacement)
                    )
                    .await
                    .unwrap()
                    .unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
            });
        child.terminate();
    });
}

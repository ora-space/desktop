use super::*;
use crate::support::{ChildGuard, until};
use pretty_assertions::assert_eq;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::{net::UnixStream, time::timeout};

/// Starts the production service with private injected IPC and a deployment-selected owner.
pub(super) fn launch(fixture: &Fixture, clone: &CloneConfig) -> ChildGuard {
    // Ordinary replay tests allow the configured 30-second Git deadline plus cleanup; dedicated
    // lifecycle tests inject shorter admission deadlines to exercise timeout and reconnection.
    launch_with_deadline(fixture, clone, /*frame_timeout_ms*/ 40_000)
}

/// Injects a session deadline without changing production defaults or environment variables.
pub(super) fn launch_with_deadline(
    fixture: &Fixture,
    clone: &CloneConfig,
    frame_timeout_ms: u64,
) -> ChildGuard {
    let config = write_config(fixture, clone, frame_timeout_ms);
    ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_ora-node"))
            .arg(config)
            .stdin(Stdio::null())
            .stdout(fs::File::create(fixture.path().join("ipc.log")).unwrap())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}

/// Writes the IPC deployment file that either this test or a hosting Controller starts Node from.
pub(super) fn write_config(
    fixture: &Fixture,
    clone: &CloneConfig,
    frame_timeout_ms: u64,
) -> PathBuf {
    let config = fixture.path().join("ipc-config.json");
    fs::write(
        &config,
        serde_json::to_vec(&ora_node::ServiceConfig {
            node: fixture.config(),
            process: fixture.process(),
            clone: Some(clone.clone()),
            control: Some(ora_node::ControlConfig {
                controller_id: ControllerId::new("owner"),
                listen: ora_node::ControlListen::Ipc {
                    path: fixture.config().home_directory.join("control.sock"),
                },
                heartbeat_ms: 100,
                frame_timeout_ms,
            }),
            recovery_interval_ms: 50,
            timezone: "Asia/Shanghai".into(),
        })
        .unwrap(),
    )
    .unwrap();
    config
}

/// Opens an actual framed stream without bypassing the server's handshake policy.
pub(super) async fn connect(path: &Path, owner: &str) -> UnixStream {
    let mut stream = UnixStream::connect(path).await.unwrap();
    write_controller_message(
        &mut stream,
        &ControllerToNodeMessage::Hello(HelloMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: Hello {
                controller_id: ControllerId::new(owner),
                supported_versions: vec![CURRENT_PROTOCOL_VERSION],
            },
        }),
    )
    .await
    .unwrap();
    stream
}

/// Bounds every expected delivery so a broken replay cannot hang acceptance tests.
async fn receive(stream: &mut UnixStream) -> Option<NodeToControllerMessage> {
    timeout(Duration::from_secs(/*secs*/ 40), read_node_message(stream))
        .await
        .unwrap()
        .unwrap()
}

/// Real IPC rejects another owner and duplicate sessions, then replays one durable clone across Node restart.
#[test]
fn local_ipc_enforces_owner_and_replays_original_clone_after_restart() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child = launch(&fixture, &config);
        until(|| endpoint.exists());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let command = request(&server, "ipc", "main");
        let event = runtime.block_on(async {
            let mut wrong = connect(&endpoint, "other").await;
            assert!(read_node_message(&mut wrong).await.unwrap().is_none());
            drop(wrong);
            let mut stream = connect(&endpoint, "owner").await;
            assert!(matches!(
                receive(&mut stream).await,
                Some(NodeToControllerMessage::HelloAccepted(_))
            ));
            let mut duplicate = UnixStream::connect(&endpoint).await.unwrap();
            assert!(read_node_message(&mut duplicate).await.unwrap().is_none());
            write_controller_message(
                &mut stream,
                &ControllerToNodeMessage::CloneRepository(command.clone()),
            )
            .await
            .unwrap();
            let mut accepted = false;
            loop {
                match receive(&mut stream).await.unwrap() {
                    NodeToControllerMessage::ExecutionStatus(status) => {
                        if !accepted {
                            assert_eq!(status.payload.state, ExecutionState::Accepted);
                            write_controller_message(
                                &mut stream,
                                &ControllerToNodeMessage::GetExecutionStatus(
                                    GetExecutionStatusMessage {
                                        protocol_version: CURRENT_PROTOCOL_VERSION,
                                        operation_id: command.operation_id.clone(),
                                        execution_id: command.execution_id.clone(),
                                        payload: GetExecutionStatus {
                                            node_id: NodeId::new("test-node"),
                                        },
                                    },
                                ),
                            )
                            .await
                            .unwrap();
                        }
                        accepted = true;
                    }
                    NodeToControllerMessage::CloneResult(event) => {
                        assert!(accepted);
                        break event;
                    }
                    NodeToControllerMessage::Heartbeat(_) => {}
                    message => panic!("unexpected {message:?}"),
                }
            }
        });
        child.kill();
        server.reject_auth.store(true, Ordering::SeqCst);
        let mut replacement = launch(&fixture, &config);
        // Readiness is emitted after binding; a probe connection would itself occupy the handshake slot.
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        runtime.block_on(async {
            let mut stream = connect(&endpoint, "owner").await;
            assert!(matches!(
                receive(&mut stream).await,
                Some(NodeToControllerMessage::HelloAccepted(_))
            ));
            loop {
                if let Some(NodeToControllerMessage::CloneResult(replayed)) =
                    receive(&mut stream).await
                {
                    assert_eq!(replayed, event);
                    write_controller_message(
                        &mut stream,
                        &ControllerToNodeMessage::EventAck(EventAckMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            operation_id: replayed.operation_id,
                            execution_id: replayed.execution_id,
                            sequence: replayed.sequence,
                            payload: EventAck {
                                node_id: NodeId::new("test-node"),
                            },
                        }),
                    )
                    .await
                    .unwrap();
                    break;
                }
            }
            // Ordered query proves the preceding Ack was processed without inventing a sequence.
            write_controller_message(
                &mut stream,
                &ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id.clone(),
                    execution_id: command.execution_id.clone(),
                    payload: GetExecutionStatus {
                        node_id: NodeId::new("test-node"),
                    },
                }),
            )
            .await
            .unwrap();
            loop {
                if let Some(NodeToControllerMessage::ExecutionStatus(status)) =
                    receive(&mut stream).await
                {
                    assert_eq!(
                        status.payload.state,
                        ExecutionState::Completed(ExecutionResult::Clone(event.payload.clone()))
                    );
                    break;
                }
            }
        });
        replacement.terminate();
        let node = Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        assert!(node.pending_events().unwrap().is_empty());
    });
}

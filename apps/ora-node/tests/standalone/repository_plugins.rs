//! Real Node service admission, execution, restart replay and exact acknowledgement over IPC.
use super::*;
use crate::support::until;
use pretty_assertions::assert_eq;

/// Waits for one non-heartbeat frame under a finite deadline.
async fn receive(stream: &mut tokio::net::UnixStream) -> NodeToControllerMessage {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let message = read_node_message(stream).await.unwrap().unwrap();
            if !matches!(message, NodeToControllerMessage::Heartbeat(_)) {
                return message;
            }
        }
    })
    .await
    .unwrap()
}

/// A removal of an absent package exercises the whole durable service without an external server.
#[test]
fn plugin_result_replays_after_restart_until_exact_ack_and_remains_queryable() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child = ipc::launch(&fixture, &config);
        until(|| endpoint.exists());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let command = RemovePluginsMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("plugin-op"),
            execution_id: ExecutionId::new("plugin-execution"),
            payload: RemovePlugins {
                spec: RemovePluginsSpec {
                    node_id: NodeId::new("test-node"),
                    plugins: vec![PluginRemoval {
                        plugin_id: PluginId::new("official/ora-space.echo"),
                        version: PluginVersion::new("1.0.0"),
                    }],
                },
            },
        };
        let event = runtime.block_on(async {
            let mut stream = ipc::connect(&endpoint, "owner").await;
            let NodeToControllerMessage::HelloAccepted(hello) = receive(&mut stream).await else {
                panic!("missing hello")
            };
            assert!(
                hello
                    .payload
                    .capabilities
                    .contains(&NodeCapability::PluginInstall)
            );
            write_controller_message(
                &mut stream,
                &ControllerToNodeMessage::RemovePlugins(command.clone()),
            )
            .await
            .unwrap();
            assert!(matches!(
                receive(&mut stream).await,
                NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
                    payload: ExecutionStatus {
                        state: ExecutionState::Accepted,
                        ..
                    },
                    ..
                })
            ));
            let event = receive(&mut stream).await;
            assert!(matches!(
                &event,
                NodeToControllerMessage::PluginsResult(PluginsResultMessage {
                    payload: PluginExecutionResult::PluginsCompleted(_),
                    ..
                })
            ));
            event
        });
        child.kill();
        let mut replacement = ipc::launch(&fixture, &config);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        runtime.block_on(async {
            let mut stream = ipc::connect(&endpoint, "owner").await;
            let NodeToControllerMessage::HelloAccepted(hello) = receive(&mut stream).await else {
                panic!("missing hello")
            };
            assert_eq!(receive(&mut stream).await, event);
            let NodeToControllerMessage::PluginsResult(event) = event else {
                unreachable!()
            };
            write_controller_message(
                &mut stream,
                &ControllerToNodeMessage::EventAck(EventAckMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id.clone(),
                    execution_id: command.execution_id.clone(),
                    sequence: event.sequence,
                    payload: EventAck {
                        node_id: hello.payload.node.node_id.clone(),
                    },
                }),
            )
            .await
            .unwrap();
            write_controller_message(
                &mut stream,
                &ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id.clone(),
                    execution_id: command.execution_id.clone(),
                    payload: GetExecutionStatus {
                        node_id: hello.payload.node.node_id.clone(),
                    },
                }),
            )
            .await
            .unwrap();
            assert_eq!(
                receive(&mut stream).await,
                NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id,
                    execution_id: command.execution_id,
                    payload: ExecutionStatus {
                        node: hello.payload.node,
                        state: ExecutionState::Completed(ExecutionResult::Plugin(event.payload))
                    }
                })
            );
        });
        replacement.terminate();
        let db = ora_node_db::NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        assert!(db.pending_events().unwrap().is_empty());
    });
}

/// A blocked download leaves control queries live; an unfenced restart reruns the original input.
#[test]
fn killed_download_recovers_original_input_and_cleans_temporary_package() {
    ora_logging::with_trace_logging(|| {
        use std::{
            io::Write,
            sync::{Arc, atomic::AtomicUsize},
        };
        let fixture = Fixture::new();
        let archive = fixture.path().join("release.orax");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        for (name, contents) in [
            (
                "orax.toml",
                "resolver = 1\nidentifier = \"ora-space.echo\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"test\"\n",
            ),
            ("main.js", "export {};\n"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let bytes = fs::read(&archive).unwrap();
        let (url, server) = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/release.orax", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut stalled = None;
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut buffer = [0u8; 4096];
                    stream.read(&mut buffer).await.unwrap();
                    if requests.fetch_add(1, Ordering::SeqCst) == 0 {
                        stalled = Some(stream);
                    } else {
                        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).as_bytes()).await.unwrap();
                        stream.write_all(&bytes).await.unwrap();
                        drop(stalled.take());
                    }
                }
            });
            (url, task)
        });
        let https = GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &https);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child = ipc::launch(&fixture, &config);
        until(|| endpoint.exists());
        let command = InstallPluginsMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("install-op"),
            execution_id: ExecutionId::new("install-execution"),
            payload: InstallPlugins {
                spec: InstallPluginsSpec {
                    node_id: NodeId::new("test-node"),
                    plugins: vec![PluginInstall {
                        plugin_id: PluginId::new("official/ora-space.echo"),
                        version: PluginVersion::new("1.0.0"),
                        release: PluginRelease::Universal {
                            download: PluginDownload {
                                url,
                                sha256: Sha256Digest::new(
                                    ora_utils::hash::sha256_file(&archive).unwrap(),
                                ),
                            },
                        },
                    }],
                },
            },
        };
        runtime.block_on(async {
            let mut stream = ipc::connect(&endpoint, "owner").await;
            let _hello = receive(&mut stream).await;
            write_controller_message(
                &mut stream,
                &ControllerToNodeMessage::InstallPlugins(command.clone()),
            )
            .await
            .unwrap();
            let _accepted = receive(&mut stream).await;
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while count.load(Ordering::SeqCst) != 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
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
            assert!(matches!(
                receive(&mut stream).await,
                NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
                    payload: ExecutionStatus {
                        state: ExecutionState::Running,
                        ..
                    },
                    ..
                })
            ));
        });
        child.kill();
        let staging = fixture
            .config()
            .home_directory
            .join("plugins")
            .join(".node-installs");
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 1);
        let mut replacement = ipc::launch(&fixture, &config);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        runtime.block_on(async {
            let mut stream = ipc::connect(&endpoint, "owner").await;
            let _hello = receive(&mut stream).await;
            let NodeToControllerMessage::PluginsResult(event) = receive(&mut stream).await else {
                panic!("missing recovered result")
            };
            assert_eq!(
                (event.operation_id, event.execution_id, event.sequence),
                (
                    command.operation_id,
                    command.execution_id,
                    Sequence::new(/*value*/ 1)
                )
            );
            let PluginExecutionResult::PluginsCompleted(result) = event.payload else {
                panic!("installation failed")
            };
            assert_eq!(
                result.items,
                vec![PluginItemResult {
                    plugin_id: PluginId::new("official/ora-space.echo"),
                    outcome: PluginItemOutcome::Installed {
                        version: PluginVersion::new("1.0.0")
                    }
                }]
            );
        });
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(fs::read_dir(staging).unwrap().count(), 0);
        replacement.terminate();
        server.abort();
    });
}

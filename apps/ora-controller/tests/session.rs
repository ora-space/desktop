#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
use ora_controller::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};
use tokio::{net::UnixListener, time::timeout};

/// A reachable socket is not sufficient authority to dispatch a previously accepted command.
#[test]
fn mismatched_node_or_missing_clone_capability_rejects_before_dispatch() {
    ora_logging::with_trace_logging(|| {
        for (node_id, capabilities) in [
            (
                NodeId::new("other-node"),
                vec![NodeCapability::RepositoryClone],
            ),
            (NodeId::new("node"), vec![NodeCapability::WorktreeExecution]),
        ] {
            let root = tempfile::Builder::new()
                .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
                .tempdir_in(std::env::var_os("HOME").unwrap())
                .unwrap();
            let store =
                SqliteStore::open(&root.path().join("controller"), ControllerId::new("owner"))
                    .unwrap();
            fs::create_dir(root.path().join("node")).unwrap();
            let socket = root.path().join("node").join("control.sock");
            let target = NodeTarget {
                node_id: NodeId::new("node"),
                endpoint: NodeEndpoint::Ipc {
                    path: socket.clone(),
                },
            };
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let command = store
                        .accept_request(
                            RequestId::new("request"),
                            CloneExecutionSpec {
                                node_id: NodeId::new("node"),
                                repository: CloneRepositoryUrl::parse("https://example.com/repo")
                                    .unwrap(),
                                branch: BranchName::new("main"),
                            },
                        )
                        .await
                        .unwrap();
                    let listener = UnixListener::bind(&socket).unwrap();
                    let settings = SessionConfig {
                        io_timeout_ms: 1000,
                        query_interval_ms: 20,
                    };
                    let peer = async {
                        let (mut stream, _) = listener.accept().await.unwrap();
                        assert!(matches!(
                            read_controller_message(&mut stream).await.unwrap(),
                            Some(ControllerToNodeMessage::Hello(_))
                        ));
                        write_node_message(
                            &mut stream,
                            &NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                payload: HelloAccepted {
                                    selected_version: CURRENT_PROTOCOL_VERSION,
                                    node: NodeRuntimeIdentity {
                                        node_id,
                                        incarnation_id: NodeIncarnationId::new("current"),
                                    },
                                    capabilities,
                                },
                            }),
                        )
                        .await
                        .unwrap();
                        // EOF, not a status query or clone command, proves rejection preceded dispatch.
                        assert_eq!(
                            timeout(
                                Duration::from_secs(/*secs*/ 2),
                                read_controller_message(&mut stream)
                            )
                            .await
                            .unwrap()
                            .unwrap(),
                            None
                        );
                    };
                    let (result, ()) = tokio::join!(run_session(&store, &target, &settings), peer);
                    assert!(result.is_err());
                    assert_eq!(
                        store.pending_dispatches(&target.node_id).await.unwrap(),
                        vec![command.clone()]
                    );
                    assert_eq!(store.result(&command.execution_id).await.unwrap(), None);
                });
        }
    });
}

/// Repeated Unknown replies retain responsibility without an unbounded immediate retransmission loop.
#[test]
fn uncertain_execution_retransmits_at_most_once_per_connection() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let store =
            SqliteStore::open(&root.path().join("controller"), ControllerId::new("owner")).unwrap();
        fs::create_dir(root.path().join("node")).unwrap();
        let socket = root.path().join("node").join("control.sock");
        let endpoint = NodeTarget {
            node_id: NodeId::new("node"),
            endpoint: NodeEndpoint::Ipc {
                path: socket.clone(),
            },
        };
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let command = store
                .accept_request(
                    RequestId::new("request"),
                    CloneExecutionSpec {
                        node_id: NodeId::new("node"),
                        repository: CloneRepositoryUrl::parse("https://example.com/repo").unwrap(),
                        branch: BranchName::new("main"),
                    },
                )
                .await
                .unwrap();
            let listener = UnixListener::bind(&socket).unwrap();
            let settings = SessionConfig { io_timeout_ms: 1000, query_interval_ms: 20 };
            let session = run_session(&store, &endpoint, &settings);
            let peer = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                assert!(matches!(read_controller_message(&mut stream).await.unwrap(), Some(ControllerToNodeMessage::Hello(_))));
                let identity = NodeRuntimeIdentity { node_id: endpoint.node_id.clone(), incarnation_id: NodeIncarnationId::new("current") };
                write_node_message(&mut stream, &NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage { protocol_version: CURRENT_PROTOCOL_VERSION, payload: HelloAccepted { selected_version: CURRENT_PROTOCOL_VERSION, node: identity.clone(), capabilities: vec![NodeCapability::RepositoryClone] } })).await.unwrap();
                let mut retries = 0;
                let mut queries = 0;
                while queries < 4 {
                    let message = timeout(Duration::from_secs(/*secs*/ 2), read_controller_message(&mut stream)).await.unwrap().unwrap().unwrap();
                    match message {
                        ControllerToNodeMessage::CloneRepository(retry) => { assert_eq!(retry, command); retries += 1; assert!(retries <= 1); }
                        ControllerToNodeMessage::GetExecutionStatus(_) => { queries += 1; }
                        message => panic!("unexpected {message:?}"),
                    }
                    write_node_message(&mut stream, &NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage { protocol_version: CURRENT_PROTOCOL_VERSION, operation_id: command.operation_id.clone(), execution_id: command.execution_id.clone(), payload: ExecutionStatus { node: identity.clone(), state: ExecutionState::Unknown } })).await.unwrap();
                }
                assert_eq!(retries, 1);
            };
            tokio::select! { _ = session => panic!("session ended before peer checks"), _ = peer => {} }
            assert_eq!(store.result(&command.execution_id).await.unwrap(), None);
        });
    });
}

/// Connection failures are classified and keep every accepted dispatch pending for reconnection.
#[test]
fn connection_failures_are_classified_without_releasing_responsibility() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let store =
            SqliteStore::open(&root.path().join("controller"), ControllerId::new("owner")).unwrap();
        let settings = SessionConfig {
            io_timeout_ms: 1000,
            query_interval_ms: 20,
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let command = store
                    .accept_request(
                        RequestId::new("request"),
                        CloneExecutionSpec {
                            node_id: NodeId::new("node"),
                            repository: CloneRepositoryUrl::parse("https://example.com/repo")
                                .unwrap(),
                            branch: BranchName::new("main"),
                        },
                    )
                    .await
                    .unwrap();
                // Reserve and release a port so nothing listens there.
                let closed = std::net::TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap();
                for endpoint in [
                    NodeEndpoint::Ipc {
                        path: root.path().join("missing.sock"),
                    },
                    NodeEndpoint::WebSocket(ora_node_transport::websocket::WsEndpoint {
                        url: format!("ws://{closed}/ora-node/v1"),
                        headers: Default::default(),
                    }),
                ] {
                    let target = NodeTarget {
                        node_id: NodeId::new("node"),
                        endpoint,
                    };
                    let error = run_session(&store, &target, &settings).await.unwrap_err();
                    assert!(
                        matches!(
                            &error,
                            SessionError::Connect(connect)
                                if connect.failure == ora_node_transport::ConnectFailure::Unreachable
                        ),
                        "{error:?}"
                    );
                }
                assert_eq!(
                    store.pending_dispatches(&NodeId::new("node")).await.unwrap(),
                    vec![command.clone()]
                );
                assert_eq!(store.result(&command.execution_id).await.unwrap(), None);
            });
    });
}

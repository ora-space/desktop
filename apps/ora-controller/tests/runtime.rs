#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
use ora_controller::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt};

/// The embedding interface preserves durable intent across runtime shutdown and excludes duplicate owners.
#[test]
fn embedded_owner_reopens_original_operations_and_rejects_overlap() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let config = RuntimeConfig {
            home_directory: root.path().join("controller"),
            persistence: Persistence::Sqlite,
            protected_state_directories: vec![root.path().join("process")],
            controller_id: ControllerId::new("owner"),
            nodes: vec![NodeTarget {
                node_id: NodeId::new("node"),
                endpoint: NodeEndpoint::Ipc {
                    path: root.path().join("node").join("control.sock"),
                },
            }],
            session: SessionConfig {
                io_timeout_ms: 100,
                query_interval_ms: 10,
            },
            reconnect_ms: 10,
            timezone: "Asia/Shanghai".into(),
        };
        let mut overlap = config.clone();
        overlap.home_directory = root.path().join("process").join("nested");
        assert!(ControllerRuntime::<SqliteStore>::open(overlap).is_err());
        assert!(!root.path().join("process").exists());
        // Each adapter opens only its own persistence kind: neither is a fallback for the other.
        let mut cloud = config.clone();
        cloud.persistence = Persistence::Cloud {
            endpoint: "http://127.0.0.1:1".into(),
            claim_interval_ms: 1000,
        };
        assert!(matches!(
            ControllerRuntime::<SqliteStore>::open(cloud.clone()),
            Err(Error::Configuration(_))
        ));
        assert!(matches!(
            ControllerRuntime::<CloudStore>::open(config.clone()),
            Err(Error::Configuration(_))
        ));
        // Cloud persistence dispatches to one Node and never creates local state, even when run.
        let mut two_nodes = cloud.clone();
        two_nodes.nodes.push(NodeTarget {
            node_id: NodeId::new("second"),
            endpoint: NodeEndpoint::Ipc {
                path: root.path().join("second").join("control.sock"),
            },
        });
        assert!(matches!(
            ControllerRuntime::<CloudStore>::open(two_nodes),
            Err(Error::Configuration(_))
        ));
        let mut bad_endpoint = cloud.clone();
        if let Persistence::Cloud { endpoint, .. } = &mut bad_endpoint.persistence {
            *endpoint = "not a uri".into();
        }
        assert!(matches!(
            ControllerRuntime::<CloudStore>::open(bad_endpoint),
            Err(Error::Configuration(_))
        ));
        // A WebSocket endpoint that can never connect is a deployment error, not endless reconnects.
        for url in ["http://127.0.0.1:1/ora-node/v1", "not a url"] {
            let mut websocket = config.clone();
            websocket.nodes[0].endpoint =
                NodeEndpoint::WebSocket(ora_node_transport::websocket::WsEndpoint {
                    url: url.into(),
                    headers: Default::default(),
                });
            assert!(matches!(
                ControllerRuntime::<SqliteStore>::open(websocket),
                Err(Error::Conflict)
            ));
        }
        assert!(!root.path().join("controller").exists());
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let remote = ControllerRuntime::<CloudStore>::open(cloud).unwrap();
                remote.run(async {}).await.unwrap();
                drop(remote);
                assert!(!root.path().join("controller").exists());
                let runtime = ControllerRuntime::<SqliteStore>::open(config.clone()).unwrap();
                assert!(matches!(
                    ControllerRuntime::<SqliteStore>::open(config.clone()),
                    Err(Error::AlreadyRunning)
                ));
                let handle = runtime.handle();
                let spec = CloneExecutionSpec {
                    node_id: NodeId::new("node"),
                    repository: CloneRepositoryUrl::parse("https://example.com/repo.git").unwrap(),
                    branch: BranchName::new("main"),
                };
                let command = handle
                    .accept_clone(RequestId::new("request"), spec.clone())
                    .await
                    .unwrap();
                assert_eq!(
                    handle
                        .accept_clone(RequestId::new("request"), spec)
                        .await
                        .unwrap(),
                    command
                );
                let expected = CloneOperation {
                    command: command.clone(),
                    result: None,
                };
                assert_eq!(handle.operations().await.unwrap(), vec![expected.clone()]);
                assert_eq!(
                    handle.operation(ExecutionId::new("absent")).await.unwrap(),
                    None
                );
                runtime.run(async {}).await.unwrap();
                drop(handle);
                drop(runtime);
                let replacement = ControllerRuntime::<SqliteStore>::open(config).unwrap();
                assert_eq!(
                    replacement
                        .handle()
                        .operation(command.execution_id)
                        .await
                        .unwrap(),
                    Some(expected)
                );
            });
    });
}

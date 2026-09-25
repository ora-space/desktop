#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
use ora_contracts::controller_api::*;
use ora_controller::{
    ApiConfig, CloneIntake, DeploymentConfig, NodeEndpoint, NodeHosting, NodeTarget, Persistence,
    RuntimeConfig, Service, SessionConfig, SingleNodeConfig, SqliteStore, Transport,
};
use ora_node_protocol::{BranchName, CloneExecutionSpec, CloneRepositoryUrl, ControllerId, NodeId};
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt};

/// Starts the composition without a hosted Node on an ephemeral loopback port.
async fn start(config: DeploymentConfig) -> Result<Service<SqliteStore>, ora_controller::Error> {
    Service::<SqliteStore>::start(
        config,
        Transport::loopback(/*port*/ 0),
        NodeHosting::External,
    )
    .await
}

/// Resolves the HTTP base of a bound service; the transitional surface only binds TCP in this test.
fn clones_url(service: &Service<SqliteStore>) -> String {
    match service.endpoint().unwrap() {
        Transport::Tcp(address) => format!("http://{address}/api/clones"),
        Transport::Unix(path) => panic!("unexpected Unix endpoint {}", path.display()),
    }
}

/// Real HTTP acceptance, conflict and restart use the same durable owner, even when Node is offline.
#[test]
fn http_acceptance_is_idempotent_and_survives_service_restart() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let config = DeploymentConfig {
            api: Some(ApiConfig {
                node_id: NodeId::new("node"),
            }),
            single_node: None,
            controller: RuntimeConfig {
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
                    query_interval_ms: 50,
                },
                reconnect_ms: 50,
                timezone: "Asia/Shanghai".into(),
            },
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                // Composition errors are rejected before any Controller state exists on disk.
                let mut unknown_target = config.clone();
                unknown_target.api = Some(ApiConfig {
                    node_id: NodeId::new("other"),
                });
                assert!(start(unknown_target).await.is_err());
                let mut no_surface = config.clone();
                no_surface.api = None;
                assert!(start(no_surface).await.is_err());
                assert!(
                    Service::<SqliteStore>::start(
                        config.clone(),
                        Transport::loopback(/*port*/ 0),
                        NodeHosting::Managed
                    )
                    .await
                    .is_err()
                );
                let mut hosted = config.clone();
                hosted.single_node = Some(SingleNodeConfig {
                    node_executable: "relative/ora-node".into(),
                    node_config: root.path().join("node.json"),
                    ready_timeout_ms: 1000,
                    stop_timeout_ms: 1000,
                });
                assert!(
                    Service::<SqliteStore>::start(
                        hosted,
                        Transport::loopback(/*port*/ 0),
                        NodeHosting::Managed
                    )
                    .await
                    .is_err()
                );
                // Hosting is only defined for exactly the one Node the API dispatches to.
                let mut many = config.clone();
                many.single_node = Some(SingleNodeConfig {
                    node_executable: root.path().join("ora-node"),
                    node_config: root.path().join("node.json"),
                    ready_timeout_ms: 1000,
                    stop_timeout_ms: 1000,
                });
                many.controller.nodes.push(NodeTarget {
                    node_id: NodeId::new("second"),
                    endpoint: NodeEndpoint::Ipc {
                        path: root.path().join("second").join("control.sock"),
                    },
                });
                assert!(
                    Service::<SqliteStore>::start(
                        many,
                        Transport::loopback(/*port*/ 0),
                        NodeHosting::Managed
                    )
                    .await
                    .is_err()
                );
                assert!(!config.controller.home_directory.exists());
                let mut overlap = config.clone();
                overlap.controller.home_directory = root.path().join("process").join("nested");
                assert!(start(overlap).await.is_err());
                assert!(!root.path().join("process").exists());
                fs::create_dir(&config.controller.home_directory).unwrap();
                fs::set_permissions(
                    &config.controller.home_directory,
                    fs::Permissions::from_mode(/*mode*/ 0o700),
                )
                .unwrap();
                let unknown = config
                    .controller
                    .home_directory
                    .join("ora-controller.sqlite3");
                fs::write(&unknown, b"user-owned unknown file").unwrap();
                assert!(start(config.clone()).await.is_err());
                assert_eq!(fs::read(&unknown).unwrap(), b"user-owned unknown file");
                // Retain the rejected fixture file; the legitimate owner starts in a fresh root.
                let mut config = config;
                config.controller.home_directory = root.path().join("valid-controller");
                let standalone = ora_controller::SqliteStore::open(
                    &config.controller.home_directory,
                    config.controller.controller_id.clone(),
                )
                .unwrap();
                let original = standalone
                    .accept_request(
                        ora_node_protocol::RequestId::new("original"),
                        CloneExecutionSpec {
                            node_id: NodeId::new("node"),
                            repository: CloneRepositoryUrl::parse("https://example.com/repo.git")
                                .unwrap(),
                            branch: BranchName::new("main"),
                        },
                    )
                    .await
                    .unwrap();
                assert!(start(config.clone()).await.is_err());
                drop(standalone);
                let service = start(config.clone()).await.unwrap();
                assert!(start(config.clone()).await.is_err());
                // A Unix API socket outside the Controller home is refused while the owner is held.
                let outside = Transport::Unix(root.path().join("api.sock"));
                assert!(
                    outside
                        .bind(&config.controller.home_directory)
                        .await
                        .is_err()
                );
                // An existing regular file or symlink at the socket path is never replaced.
                let occupied = config.controller.home_directory.join("occupied.sock");
                fs::write(&occupied, b"user file").unwrap();
                assert!(
                    Transport::Unix(occupied.clone())
                        .bind(&config.controller.home_directory)
                        .await
                        .is_err()
                );
                assert_eq!(fs::read(&occupied).unwrap(), b"user file");
                let linked = config.controller.home_directory.join("linked.sock");
                std::os::unix::fs::symlink(&occupied, &linked).unwrap();
                assert!(
                    Transport::Unix(linked.clone())
                        .bind(&config.controller.home_directory)
                        .await
                        .is_err()
                );
                assert!(
                    fs::symlink_metadata(&linked)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                assert_eq!(fs::read(&occupied).unwrap(), b"user file");
                let base = clones_url(&service);
                let (stop, stopped) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(service.run(async {
                    let _ = stopped.await;
                }));
                let client = reqwest::Client::new();
                let input = MiniCloneRequest {
                    request_id: "original".into(),
                    repository: "https://example.com/repo.git".into(),
                    branch: "main".into(),
                };
                let response = client.post(&base).json(&input).send().await.unwrap();
                assert_eq!(response.status().as_u16(), 202);
                let accepted: MiniCloneAccepted = response.json().await.unwrap();
                assert_eq!(
                    accepted,
                    MiniCloneAccepted {
                        request_id: "original".into(),
                        operation_id: original.operation_id.as_str().into(),
                        execution_id: original.execution_id.as_str().into(),
                    }
                );
                assert_eq!(
                    client
                        .post(&base)
                        .json(&input)
                        .send()
                        .await
                        .unwrap()
                        .json::<MiniCloneAccepted>()
                        .await
                        .unwrap(),
                    accepted
                );
                let mut changed = input.clone();
                changed.branch = "other".into();
                assert_eq!(
                    client
                        .post(&base)
                        .json(&changed)
                        .send()
                        .await
                        .unwrap()
                        .status()
                        .as_u16(),
                    409
                );
                changed.request_id = "bad".into();
                changed.repository = "file:///tmp/repository".into();
                assert_eq!(
                    client
                        .post(&base)
                        .json(&changed)
                        .send()
                        .await
                        .unwrap()
                        .status()
                        .as_u16(),
                    400
                );
                assert_eq!(
                    client
                        .get(format!("{base}/missing"))
                        .send()
                        .await
                        .unwrap()
                        .status()
                        .as_u16(),
                    404
                );
                let expected = MiniCloneOperation {
                    operation_id: accepted.operation_id,
                    execution_id: accepted.execution_id.clone(),
                    node_id: "node".into(),
                    repository: input.repository,
                    branch: input.branch,
                    state: MiniCloneState::Pending,
                };
                assert_eq!(
                    client
                        .get(&base)
                        .send()
                        .await
                        .unwrap()
                        .json::<Vec<MiniCloneOperation>>()
                        .await
                        .unwrap(),
                    vec![expected.clone()]
                );
                stop.send(()).unwrap();
                task.await.unwrap().unwrap();
                // The same surface is reachable over a private Unix socket inside the Controller home.
                let service = Service::<SqliteStore>::start(
                    config.clone(),
                    Transport::Unix(config.controller.home_directory.join("api.sock")),
                    NodeHosting::External,
                )
                .await
                .unwrap();
                let Transport::Unix(socket) = service.endpoint().unwrap() else {
                    panic!("expected a Unix endpoint");
                };
                let (stop, stopped) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(service.run(async {
                    let _ = stopped.await;
                }));
                let listed: Vec<MiniCloneOperation> =
                    serde_json::from_slice(&unix_get(&socket, "/api/clones").await).unwrap();
                assert_eq!(listed, vec![expected.clone()]);
                stop.send(()).unwrap();
                task.await.unwrap().unwrap();
                let service = start(config).await.unwrap();
                let base = clones_url(&service);
                let (stop, stopped) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(service.run(async {
                    let _ = stopped.await;
                }));
                assert_eq!(
                    client
                        .get(format!("{base}/{}", accepted.execution_id))
                        .send()
                        .await
                        .unwrap()
                        .json::<MiniCloneOperation>()
                        .await
                        .unwrap(),
                    expected
                );
                stop.send(()).unwrap();
                task.await.unwrap().unwrap();
            });
    });
}

/// Issues one HTTP/1.1 GET over a Unix socket and returns the body; reqwest has no Unix connector here.
async fn unix_get(socket: &std::path::Path, path: &str) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200"));
    let start = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    response[start..].to_vec()
}

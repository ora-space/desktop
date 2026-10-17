#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
#[path = "support/local_clone.rs"]
mod local_clone;
use ora_controller::{
    CoordinationStore, DeploymentConfig, NodeEndpoint, NodeHosting, NodeTarget, Persistence,
    RuntimeConfig, Service, SessionConfig, SingleNodeConfig, SqliteStore,
};
use ora_node_protocol::{ControllerId, NodeId};
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt};

/// Starts the local composition without a hosted Node.
async fn start(config: DeploymentConfig) -> Result<Service<SqliteStore>, ora_controller::Error> {
    Service::<SqliteStore>::start(config, NodeHosting::External).await
}

/// Composition refusals leave no state behind, unknown files are never adopted, and the local owner
/// is exclusive across the embedding store and the composed service.
#[test]
fn local_composition_refuses_before_state_and_holds_an_exclusive_owner() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let config = DeploymentConfig {
            single_node: None,
            controller: RuntimeConfig {
                management_tls: None,
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
                assert!(
                    Service::<SqliteStore>::start(config.clone(), NodeHosting::Managed)
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
                    Service::<SqliteStore>::start(hosted, NodeHosting::Managed)
                        .await
                        .is_err()
                );
                // Hosting is only defined for a deployment with exactly one static Node.
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
                    Service::<SqliteStore>::start(many, NodeHosting::Managed)
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
                let original = local_clone::seed(&config.controller.home_directory, "original");
                // The owner is exclusive: a held database refuses a composition, and a running
                // composition refuses a second one.
                assert!(start(config.clone()).await.is_err());
                drop(standalone);
                let service = start(config.clone()).await.unwrap();
                assert!(start(config.clone()).await.is_err());
                let (stop, stopped) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(service.run(async {
                    let _ = stopped.await;
                }));
                stop.send(()).unwrap();
                task.await.unwrap().unwrap();
                // A stopped composition released the lease and kept the held dispatch.
                let reopened = ora_controller::SqliteStore::open(
                    &config.controller.home_directory,
                    config.controller.controller_id.clone(),
                )
                .unwrap();
                assert_eq!(
                    reopened
                        .pending_dispatches(&NodeId::new("node"))
                        .await
                        .unwrap(),
                    vec![original]
                );
            });
    });
}

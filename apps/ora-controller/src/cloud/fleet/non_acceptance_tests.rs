//! Drives production session polling, action delivery and the real Fleet observer over private IPC.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::cloud::{lease, tests::config};
use std::{fs, os::unix::fs::PermissionsExt};
use tokio::net::UnixListener;
#[allow(dead_code)]
#[path = "../../../tests/support/fake_cloud.rs"]
mod fake_cloud;
#[path = "../../../tests/support/non_acceptance_fixture.rs"]
mod fixture;

/// Creates the same state owner the Fleet keeps between transport connections.
fn sandbox() -> Arc<Sandbox> {
    let observed = Arc::new(Sandbox {
        binding: Binding {
            sandbox_id: "sandbox".into(),
            generation: 3,
            node_id: NodeId::new("node"),
            external_id: "external".into(),
        },
        identity: watch::Sender::new(None),
        work_hint: Arc::default(),
        report: sync::Mutex::default(),
        gate: sync::Mutex::new(Gate::Closed),
        unresolved: Mutex::default(),
        capabilities: Mutex::default(),
        model_adapter: Mutex::default(),
        known: Mutex::default(),
    });
    observed.unresolved(&ExecutionId::new("execution"));
    observed
}

/// Answers the production session's negotiated protocol; an ordinary ACK for a proof fails.
async fn peer(listener: UnixListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let hello = read_controller_message(&mut socket).await.unwrap().unwrap();
    assert!(matches!(
        hello,
        ControllerToNodeMessage::Hello(HelloMessage {
            payload: Hello {
                execution_non_acceptance: true,
                ..
            },
            ..
        })
    ));
    let node = NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("original-incarnation"),
    };
    write_node_message(
        &mut socket,
        &NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: HelloAccepted {
                thread_previews: false,
                selected_version: CURRENT_PROTOCOL_VERSION,
                node: node.clone(),
                capabilities: vec![
                    NodeCapability::RepositoryClone,
                    NodeCapability::RuntimeControl,
                    NodeCapability::ExecutionNonAcceptance,
                ],
                model_adapter: None,
            },
        }),
    )
    .await
    .unwrap();
    while let Ok(Some(message)) = read_controller_message(&mut socket).await {
        let reply = match message {
            ControllerToNodeMessage::Heartbeat(_) => {
                Some(NodeToControllerMessage::Heartbeat(HeartbeatMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    payload: Heartbeat { node: node.clone() },
                }))
            }
            ControllerToNodeMessage::FenceUnacceptedExecution(fence) => Some(
                NodeToControllerMessage::ExecutionNotAccepted(Box::new(ExecutionNonAcceptance {
                    proof_id: "11111111-1111-4111-8111-111111111111".into(),
                    fence,
                })),
            ),
            ControllerToNodeMessage::EventAck(_) => {
                panic!("non-acceptance proof must never produce an event ACK")
            }
            other => panic!("unexpected proof-session command: {other:?}"),
        };
        if let Some(reply) = reply
            && write_node_message(&mut socket, &reply).await.is_err()
        {
            break;
        }
    }
}

/// Session/control → session → Fleet clears an old Unknown only from explicit committed proof.
#[test]
fn reconnect_committed_proof_clears_the_real_fleet_unknown() {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cloud = fake_cloud::FakeCloud::new();
                let origin = fixture::fence();
                cloud.seed_execution_fence(origin.clone());
                cloud.seed_non_acceptance_proof(proto::ExecutionNonAcceptance {
                    proof_id: "11111111-1111-4111-8111-111111111111".into(),
                    fence: Some(origin),
                });
                let served = cloud.serve().await;
                let mut settings = config("owner");
                if let Persistence::Cloud { endpoint, .. } = &mut settings.persistence {
                    endpoint.clone_from(&served.endpoint);
                }
                let store = CloudStore::open(&settings).unwrap();
                lease::keep_lease(&store).await;
                let observed = sandbox();
                assert!(observed.any_unresolved());
                let root = tempfile::Builder::new()
                    .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
                    .tempdir_in(std::env::var_os("HOME").unwrap())
                    .unwrap();
                let path = root.path().join("node.sock");
                let listener = UnixListener::bind(&path).unwrap();
                let target = NodeTarget {
                    node_id: NodeId::new("node"),
                    endpoint: NodeEndpoint::Ipc { path },
                };
                let stop = async {
                    tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
                        while observed.any_unresolved() {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .expect("committed proof did not clear the original Unknown");
                };
                let (result, ()) = tokio::join!(
                    run_observed_session(
                        &store,
                        &target,
                        &SessionConfig {
                            io_timeout_ms: 2000,
                            query_interval_ms: 10
                        },
                        stop,
                        observed.as_ref()
                    ),
                    peer(listener)
                );
                result.unwrap();
                assert!(!observed.any_unresolved());
            });
    });
}

/// A failed real proof RPC ends this connection without clearing Fleet responsibility or ACKing.
#[test]
fn failed_proof_takeover_keeps_the_real_fleet_unknown() {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cloud = fake_cloud::FakeCloud::new();
                cloud.seed_execution_fence(fixture::fence());
                cloud.reject_execution_proofs();
                let served = cloud.serve().await;
                let mut settings = config("owner");
                if let Persistence::Cloud { endpoint, .. } = &mut settings.persistence {
                    endpoint.clone_from(&served.endpoint);
                }
                let store = CloudStore::open(&settings).unwrap();
                lease::keep_lease(&store).await;
                let observed = sandbox();
                let root = tempfile::Builder::new()
                    .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
                    .tempdir_in(std::env::var_os("HOME").unwrap())
                    .unwrap();
                let path = root.path().join("node.sock");
                let listener = UnixListener::bind(&path).unwrap();
                let target = NodeTarget {
                    node_id: NodeId::new("node"),
                    endpoint: NodeEndpoint::Ipc { path },
                };
                let session = run_observed_session(
                    &store,
                    &target,
                    &SessionConfig {
                        io_timeout_ms: 2000,
                        query_interval_ms: 10,
                    },
                    std::future::pending(),
                    observed.as_ref(),
                );
                let (result, ()) = tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
                    tokio::join!(session, peer(listener))
                })
                .await
                .expect("failed proof takeover did not end the session");
                assert!(result.is_err());
                cloud.until(|state| state.non_acceptance_attempts > 0).await;
                assert!(observed.any_unresolved());
            });
    });
}

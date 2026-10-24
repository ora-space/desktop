//! Behavior of the Cloud persistence adapter's coordination loop against an in-memory Cloud that
//! serves the real contract: signal-driven claiming, fallback and reopening, drains, stale
//! eligibility and shutdown, and claiming only once the static Node proved its identity. Claimed
//! work without a target sandbox is refused, so nothing here is ever registered for dispatch.
//!
//! Spec: specs/test-cases/controller/api-boundary/watch-signal-claiming.md
//! Decision: specs/decisions/controller/node-management/20260926-static-node-identity-proven-before-dispatch.md
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

#[path = "support/fake_cloud.rs"]
mod fake_cloud;

use fake_cloud::{Call, FakeCloud, State, WatchPolicy};
use ora_controller::*;
use ora_node_protocol::*;
use ora_node_transport::{Acceptor, FrameReceiver, FrameSender, ipc::IpcAcceptor};
use pretty_assertions::assert_eq;
use std::{
    fs,
    future::Future,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::UnixListener, sync::oneshot, task::JoinHandle};

/// The first lease the fake grants; every call below runs under it unless a test drops it.
const EPOCH: i64 = 1;
/// An interval no test outlives: claims can only come from the stream or its establishment.
const NEVER: u64 = 3_600_000;
/// A short fallback interval so tests observe several claim ticks quickly.
const FAST: u64 = 50;

/// A Controller running in cloud persistence mode against the fake.
struct Controller {
    stop: oneshot::Sender<()>,
    running: JoinHandle<std::io::Result<()>>,
}

impl Controller {
    /// Stops the runtime the way the executable does and waits for it to finish.
    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.running.await.unwrap().unwrap();
    }
}

/// The static Node on the configured socket. It does not listen until [`Node::listen`], so a test
/// decides when the handshake that lets the Controller claim tenant work can happen. It presents
/// the configured `node` unless a test makes it present another identity.
#[derive(Clone)]
struct Node {
    socket: PathBuf,
    identity: Arc<Mutex<NodeId>>,
    hellos: Arc<AtomicUsize>,
}

impl Node {
    /// Starts accepting sessions. Each session answers status queries with `Unknown` and keeps
    /// sending heartbeats; nothing is ever executed.
    fn listen(&self) {
        let acceptor = IpcAcceptor::new(UnixListener::bind(&self.socket).unwrap());
        let node = self.clone();
        tokio::spawn(async move {
            while let Ok(pending) = acceptor.accept().await {
                if let Ok((receiver, sender)) = acceptor.open(pending).await {
                    node.session(receiver, sender).await;
                }
            }
        });
    }

    /// Makes later handshakes present `node_id`.
    fn present(&self, node_id: &str) {
        *self.identity.lock().unwrap() = NodeId::new(node_id);
    }

    /// Waits until the Controller has sent `count` handshakes in total.
    async fn until_hellos(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
            while self.hellos.load(Ordering::SeqCst) < count {
                tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
            }
        })
        .await
        .unwrap();
    }

    /// One Node session until the Controller closes it.
    async fn session<R: FrameReceiver, W: FrameSender>(&self, mut receiver: R, mut sender: W) {
        let identity = NodeRuntimeIdentity {
            node_id: self.identity.lock().unwrap().clone(),
            incarnation_id: NodeIncarnationId::new("incarnation"),
        };
        let mut greeted = false;
        // Well inside the Controller's I/O deadline, which the heartbeats keep from elapsing.
        let mut beat = tokio::time::interval(Duration::from_millis(/*millis*/ 20));
        loop {
            let reply = tokio::select! {
                frame = receiver.recv() => {
                    let Ok(Some(frame)) = frame else { return };
                    match decode_controller_frame(&frame).unwrap() {
                        ControllerToNodeMessage::Hello(_) => {
                            greeted = true;
                            self.hellos.fetch_add(1, Ordering::SeqCst);
                            Some(NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                payload: HelloAccepted { model_adapter: None, thread_previews: false,
                                    selected_version: CURRENT_PROTOCOL_VERSION,
                                    node: identity.clone(),
                                    capabilities: vec![NodeCapability::RepositoryClone, NodeCapability::RuntimeControl],
                                },
                            }))
                        }
                        ControllerToNodeMessage::GetExecutionStatus(query) => {
                            Some(NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                operation_id: query.operation_id,
                                execution_id: query.execution_id,
                                payload: ExecutionStatus {
                                    stop_unconfirmed: Vec::new(),
                                    node: identity.clone(),
                                    state: ExecutionState::Unknown,
                                },
                            }))
                        }
                        _ => None,
                    }
                }
                _ = beat.tick(), if greeted => Some(NodeToControllerMessage::Heartbeat(HeartbeatMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    payload: Heartbeat { node: identity.clone() },
                })),
            };
            if let Some(reply) = reply
                && sender
                    .send(encode_node_frame(&reply).unwrap())
                    .await
                    .is_err()
            {
                return;
            }
        }
    }
}

/// Runs `test` with a fake Cloud and a Controller claiming every `claim_interval_ms` without a
/// stream. The configured Node is `node`; it is unreachable until the test lets it listen, and the
/// session then only reads from the store, so every recorded call comes from the coordination loop.
fn scenario<F, Fut>(claim_interval_ms: u64, test: F)
where
    F: FnOnce(FakeCloud, Box<dyn FnOnce(&str) -> Controller>, Node) -> Fut,
    Fut: Future<Output = ()>,
{
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let home = root.path().join("controller");
        let socket = root.path().join("node").join("control.sock");
        fs::create_dir(root.path().join("node")).unwrap();
        let node = Node {
            socket: socket.clone(),
            identity: Arc::new(Mutex::new(NodeId::new("node"))),
            hellos: Arc::default(),
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cloud = FakeCloud::new();
                let served = cloud.serve().await;
                let endpoint = served.endpoint.clone();
                let start = Box::new(move |controller_id: &str| {
                    let config = RuntimeConfig {
                        management_tls: None,
                        home_directory: home,
                        persistence: Persistence::Cloud {
                            endpoint,
                            claim_interval_ms,
                            substrate: None,
                        },
                        protected_state_directories: Vec::new(),
                        controller_id: ControllerId::new(controller_id),
                        nodes: vec![NodeTarget {
                            node_id: NodeId::new("node"),
                            endpoint: NodeEndpoint::Ipc { path: socket },
                        }],
                        session: SessionConfig {
                            io_timeout_ms: 100,
                            query_interval_ms: 10,
                        },
                        // Short, so a Node that starts listening is reached within a few ticks.
                        reconnect_ms: 20,
                        timezone: "Asia/Shanghai".into(),
                    };
                    let runtime = ControllerRuntime::<CloudStore>::open(config).unwrap();
                    let (stop, stopped) = oneshot::channel::<()>();
                    let running = tokio::spawn(async move {
                        runtime
                            .run(async {
                                let _ = stopped.await;
                            })
                            .await
                    });
                    Controller { stop, running }
                });
                test(cloud, start, node).await;
                drop(served);
            });
        // Cloud persistence never creates local state.
        assert!(!root.path().join("controller").exists());
    });
}

/// Counts recorded calls of one kind.
fn count(calls: &[Call], call: &Call) -> usize {
    calls.iter().filter(|recorded| *recorded == call).count()
}

/// Waits until the stream is established, lets the Node complete its handshake, and waits for the
/// claim that proof triggers.
async fn established(cloud: &FakeCloud, node: &Node) {
    cloud.until(State::watching).await;
    node.listen();
    cloud
        .until(|state| state.calls.contains(&Call::ClaimWork { epoch: EPOCH }))
        .await;
}

/// A `WorkAvailable` signal claims at once, with no periodic claim in between. Work without a
/// target sandbox, the shape tenant-level clones had, is refused: nothing is registered for it.
#[test]
fn signals_claim_without_waiting_for_the_claim_interval() {
    scenario(NEVER, |cloud, start, node| async move {
        let controller = start("owner");
        established(&cloud, &node).await;
        let signalled = tokio::time::Instant::now();
        cloud.enqueue("op-1");
        cloud.signal_work("op-1");
        cloud.until(|state| state.claimed == ["op-1"]).await;
        assert!(signalled.elapsed() < Duration::from_secs(/*secs*/ 1));
        controller.stop().await;
        assert_eq!(
            cloud.calls(),
            vec![
                Call::AcquireLease,
                Call::Watch { epoch: EPOCH },
                Call::ClaimWork { epoch: EPOCH },
                Call::ClaimWork { epoch: EPOCH },
                Call::ReleaseLease { epoch: EPOCH },
            ]
        );
        assert_eq!(cloud.dispatched(), Vec::<String>::new());
    });
}

/// Work Cloud queued before the static Node proved its identity has no signal coming; the
/// handshake that proves it claims at once, with no periodic claim in between.
#[test]
fn work_queued_before_the_node_handshake_is_claimed_once_it_completes() {
    scenario(NEVER, |cloud, start, node| async move {
        cloud.enqueue("op-1");
        let controller = start("owner");
        cloud.until(State::watching).await;
        node.listen();
        cloud.until(|state| state.claimed == ["op-1"]).await;
        controller.stop().await;
        assert_eq!(
            cloud.calls(),
            vec![
                Call::AcquireLease,
                Call::Watch { epoch: EPOCH },
                Call::ClaimWork { epoch: EPOCH },
                Call::ReleaseLease { epoch: EPOCH },
            ]
        );
    });
}

/// A stream that breaks falls back to periodic claims, and a reopened stream claims on signals
/// again.
#[test]
fn a_broken_stream_falls_back_to_periodic_claims_and_reopens() {
    scenario(FAST, |cloud, start, node| async move {
        let controller = start("owner");
        established(&cloud, &node).await;
        cloud.answer_watch(WatchPolicy::Unavailable);
        cloud.break_stream();
        // No signal is sent: only the periodic claim can find this work.
        cloud.enqueue("op-1");
        cloud.until(|state| state.claimed == ["op-1"]).await;
        cloud.answer_watch(WatchPolicy::Accept);
        cloud.until(State::watching).await;
        cloud.enqueue("op-2");
        cloud.signal_work("op-2");
        cloud.until(|state| state.claimed == ["op-1", "op-2"]).await;
        controller.stop().await;
    });
}

/// A drain stops claiming without releasing the lease; the stream reopening on a recovered Cloud
/// claims the work queued meanwhile.
#[test]
fn a_drain_pauses_claims_until_a_new_stream_opens() {
    scenario(FAST, |cloud, start, node| async move {
        let controller = start("owner");
        established(&cloud, &node).await;
        cloud.drain();
        cloud.enqueue("op-1");
        let drained_at = cloud.calls().len();
        // Every claim tick while drained tries to reopen, so reopen attempts count claim ticks:
        // three refused reopens prove at least three ticks passed without a claim.
        cloud
            .until(|state| count(&state.calls[drained_at..], &Call::Watch { epoch: EPOCH }) >= 3)
            .await;
        let during_drain = cloud.calls()[drained_at..].to_vec();
        assert_eq!(
            during_drain
                .iter()
                .filter(|call| **call != Call::Watch { epoch: EPOCH })
                .collect::<Vec<_>>(),
            Vec::<&Call>::new()
        );
        cloud.answer_watch(WatchPolicy::Accept);
        cloud.until(|state| state.claimed == ["op-1"]).await;
        controller.stop().await;
        assert_eq!(
            count(&cloud.calls(), &Call::ReleaseLease { epoch: EPOCH }),
            1
        );
    });
}

/// A serving Cloud that refuses the stream after a drain ends the drain: claiming degrades to the
/// periodic fallback instead of stopping for good.
#[test]
fn a_refused_stream_after_a_drain_resumes_periodic_claims() {
    scenario(FAST, |cloud, start, node| async move {
        let controller = start("owner");
        established(&cloud, &node).await;
        cloud.drain();
        cloud.answer_watch(WatchPolicy::Unimplemented);
        cloud.enqueue("op-1");
        cloud.until(|state| state.claimed == ["op-1"]).await;
        controller.stop().await;
    });
}

/// A stale verdict on opening the stream drops the epoch: nothing is claimed or written until a
/// lease is acquired again, which only the next renewal would attempt.
#[test]
fn a_stale_stream_open_stops_claims_and_writes() {
    scenario(FAST, |cloud, start, node| async move {
        cloud.answer_watch(WatchPolicy::Stale);
        cloud.enqueue("op-1");
        // The Node is proven, so only the lost epoch can be what stops the claims.
        node.listen();
        let controller = start("owner");
        cloud
            .until(|state| state.calls.contains(&Call::Watch { epoch: EPOCH }))
            .await;
        // Without a lease the loop makes no call at all, so there is nothing to wait on; several
        // claim ticks must pass instead, well inside the ten-second renewal.
        tokio::time::sleep(Duration::from_millis(FAST * 6)).await;
        controller.stop().await;
        assert_eq!(
            cloud.calls(),
            vec![Call::AcquireLease, Call::Watch { epoch: EPOCH }]
        );
        assert_eq!(cloud.dispatched(), Vec::<String>::new());
    });
}

/// Shutdown cancels the stream and releases the lease under the epoch it was opened with.
#[test]
fn shutdown_cancels_the_stream_and_releases_the_lease() {
    scenario(NEVER, |cloud, start, node| async move {
        let controller = start("owner");
        established(&cloud, &node).await;
        let cancelled = cloud.clone();
        let cancelled = tokio::spawn(async move { cancelled.watch_cancelled().await });
        controller.stop().await;
        cancelled.await.unwrap();
        assert_eq!(
            cloud.calls().last(),
            Some(&Call::ReleaseLease { epoch: EPOCH })
        );
    });
}

/// A Node that answers for another identity than the configured one never lets claims start: the
/// work stays queued with Cloud through many claim ticks. Once the Node behind the endpoint
/// presents the configured identity, the next handshake proves it and the queued work is claimed
/// without a signal.
#[test]
fn tenant_work_waits_until_the_configured_node_proves_its_identity() {
    scenario(FAST, |cloud, start, node| async move {
        cloud.enqueue("op-1");
        node.present("another-node");
        node.listen();
        let controller = start("owner");
        cloud.until(State::watching).await;
        // Every refused handshake is followed by a reconnect; several of them, spanning several
        // claim ticks, prove the Controller kept trying and still registered nothing.
        node.until_hellos(3).await;
        tokio::time::sleep(Duration::from_millis(FAST * 4)).await;
        assert_eq!(
            cloud.calls(),
            vec![Call::AcquireLease, Call::Watch { epoch: EPOCH }]
        );
        node.present("node");
        cloud.until(|state| state.claimed == ["op-1"]).await;
        controller.stop().await;
    });
}

/// A persisted upload has a distinct execution family even while its Node has no local record.
fn artifact_record() -> ora_controller_proto::v1::ExecutionRecord {
    use ora_controller_proto::v1 as proto;
    proto::ExecutionRecord {
        operation_id: "run".into(),
        execution_id: "artifact-upload".into(),
        node_id: "node".into(),
        node_operation_id: "upload-attempt".into(),
        input: Some(proto::ExecutionInput {
            spec: Some(proto::execution_input::Spec::DeliverScheduledArtifacts(
                proto::DeliverScheduledArtifactsSpec {
                    session_execution_id: "ended-session".into(),
                },
            )),
        }),
        result: None,
    }
}

/// Opens the production RPC adapter without starting unrelated claim or session workers.
fn artifact_store(endpoint: &str) -> CloudStore {
    CloudStore::open(&RuntimeConfig {
        management_tls: None,
        home_directory: PathBuf::from("unused-cloud-home"),
        persistence: Persistence::Cloud {
            endpoint: endpoint.into(),
            claim_interval_ms: NEVER,
            substrate: None,
        },
        protected_state_directories: Vec::new(),
        controller_id: ControllerId::new("owner"),
        nodes: vec![NodeTarget {
            node_id: NodeId::new("node"),
            endpoint: NodeEndpoint::Ipc {
                path: PathBuf::from("unused-node.sock"),
            },
        }],
        session: SessionConfig {
            io_timeout_ms: 100,
            query_interval_ms: 10,
        },
        reconnect_ms: 20,
        timezone: "Asia/Shanghai".into(),
    })
    .unwrap()
}

/// A pending upload must not fail the clone poll that shares its Controller session.
#[tokio::test]
async fn scheduled_artifact_pending_dispatch_does_not_break_clone_poll() {
    let cloud = FakeCloud::new();
    cloud.seed_record(artifact_record());
    let served = cloud.serve().await;
    let store = artifact_store(&served.endpoint);
    let node = NodeId::new("node");
    assert_eq!(store.pending_dispatches(&node).await.unwrap(), vec![]);
    assert_eq!(
        store.pending_scheduled_artifacts(&node).await.unwrap(),
        vec![DeliverScheduledArtifactsMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("upload-attempt"),
            execution_id: ExecutionId::new("artifact-upload"),
            payload: DeliverScheduledArtifacts {
                spec: DeliverScheduledArtifactsSpec {
                    session_execution_id: ExecutionId::new("ended-session"),
                },
            },
        }]
    );
}

/// Reconnecting before acceptance, or during upload, must preserve the session and its dispatch.
#[tokio::test]
async fn scheduled_artifact_nonterminal_status_preserves_its_dispatch_family() {
    let cloud = FakeCloud::new();
    cloud.seed_record(artifact_record());
    let served = cloud.serve().await;
    let store = artifact_store(&served.endpoint);
    let session = NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("current"),
    };
    for state in [
        ExecutionState::Unknown,
        ExecutionState::Accepted,
        ExecutionState::Running,
    ] {
        let mut status = ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("upload-attempt"),
            execution_id: ExecutionId::new("artifact-upload"),
            payload: ExecutionStatus {
                node: session.clone(),
                state,
                stop_unconfirmed: Vec::new(),
            },
        };
        assert_eq!(
            take_over(
                &store,
                &session,
                &NodeToControllerMessage::ExecutionStatus(status.clone())
            )
            .await
            .unwrap(),
            None
        );
        // Recognizing a family must still reject a status for another dispatch or Node.
        let mut another_node = status.clone();
        another_node.payload.node.node_id = NodeId::new("another-node");
        assert!(matches!(
            take_over(
                &store,
                &session,
                &NodeToControllerMessage::ExecutionStatus(another_node)
            )
            .await,
            Err(Error::Conflict)
        ));
        status.operation_id = OperationId::new("another-attempt");
        assert!(matches!(
            take_over(
                &store,
                &session,
                &NodeToControllerMessage::ExecutionStatus(status)
            )
            .await,
            Err(Error::Conflict)
        ));
    }
}

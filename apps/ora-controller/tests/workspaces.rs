//! A cloud Controller with a Substrate configured drives runtime Workspace operations end to end
//! against an in-memory Cloud that serves the real contract, a fake Substrate effects interface and
//! a fake sandbox Node reached over WebSocket: sandbox creation, Node registration, the Workspace
//! clone, quiesce and termination.
//!
//! Spec: specs/test-cases/controller/node-management/workspace-sandbox-driving.md
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

#[path = "support/workspace_cloud.rs"]
mod workspace_cloud;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{Method, StatusCode},
    routing::any,
};
use ora_controller::*;
use ora_controller_proto::v1 as proto;
use ora_node_protocol::*;
use ora_node_transport::{Acceptor, FrameReceiver, FrameSender, websocket::WsAcceptor};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;
use workspace_cloud::{Event, Timeline, WORKSPACE, WorkspaceCloud};

const NODE_PATH: &str = "/ora-node/v1";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// The Node identity the Substrate reports for the Workspace, as the Sandbox Server derives it.
fn node_id() -> NodeId {
    NodeId::new(format!("workspace-{WORKSPACE}"))
}

/// A Substrate that journals effects in memory and answers like the Sandbox Server.
#[derive(Clone)]
struct Substrate {
    journal: Arc<Mutex<HashMap<String, Value>>>,
    timeline: Timeline,
}

async fn effects(
    State(substrate): State<Substrate>,
    method: Method,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    if method == Method::GET {
        let entry = substrate.journal.lock().unwrap().get(&id).cloned();
        if let Some(entry) = &entry {
            substrate.timeline.push(Event::Substrate {
                method: "GET",
                kind: kind(&entry["request"]),
            });
        }
        return match entry {
            Some(entry) => (StatusCode::OK, Json(entry)),
            None => (StatusCode::NOT_FOUND, Json(Value::Null)),
        };
    }
    let request = body.map(|Json(body)| body).unwrap_or_default();
    substrate.timeline.push(Event::Substrate {
        method: "PUT",
        kind: kind(&request),
    });
    let result = match request["kind"].as_str() {
        Some("sandbox_ensure") => {
            json!({ "sandboxInstanceId": id, "nodeId": node_id().as_str() })
        }
        Some("sandbox_terminate") => json!({ "terminated": true }),
        Some("workspace_data_delete") => json!({ "removed": true }),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "unsupported_kind" })),
            );
        }
    };
    let entry = json!({ "id": id, "externalId": id, "state": "succeeded", "request": request, "result": result });
    substrate.journal.lock().unwrap().insert(id, entry.clone());
    (StatusCode::OK, Json(entry))
}

fn kind(request: &Value) -> &'static str {
    match request["kind"].as_str() {
        Some("sandbox_ensure") => "sandbox_ensure",
        Some("sandbox_terminate") => "sandbox_terminate",
        Some("workspace_data_delete") => "workspace_data_delete",
        _ => "unsupported",
    }
}

/// A sandbox Node behind the router: it accepts one session at a time, sends heartbeats, answers
/// status queries from what it executed, and completes every clone it receives when `completes`.
#[derive(Clone)]
struct Node {
    completes: Arc<AtomicBool>,
    timeline: Timeline,
}

impl Node {
    async fn serve(self) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let acceptor = WsAcceptor::new(listener, NODE_PATH);
        let url = format!("ws://{}{NODE_PATH}", acceptor.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok(pending) = acceptor.accept().await else {
                    return;
                };
                let Ok((receiver, sender)) = acceptor.open(pending).await else {
                    continue;
                };
                self.session(receiver, sender).await;
                self.timeline.push(Event::SessionEnded);
            }
        });
        url
    }

    async fn session<R: FrameReceiver, W: FrameSender>(&self, mut receiver: R, mut sender: W) {
        let identity = NodeRuntimeIdentity {
            node_id: node_id(),
            incarnation_id: NodeIncarnationId::new("incarnation-1"),
        };
        let mut results: HashMap<ExecutionId, CloneExecutionResult> = HashMap::new();
        let mut sequence = 0;
        let mut beat = tokio::time::interval(Duration::from_millis(/*millis*/ 50));
        loop {
            let reply = tokio::select! {
                frame = receiver.recv() => {
                    let Ok(Some(frame)) = frame else { return };
                    let Ok(message) = decode_controller_frame(&frame) else { return };
                    match message {
                        ControllerToNodeMessage::Hello(_) => Some(NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            payload: HelloAccepted {
                                selected_version: CURRENT_PROTOCOL_VERSION,
                                node: identity.clone(),
                                capabilities: vec![NodeCapability::RepositoryClone],
                            },
                        })),
                        ControllerToNodeMessage::GetExecutionStatus(query) => Some(NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            operation_id: query.operation_id,
                            execution_id: query.execution_id.clone(),
                            payload: ExecutionStatus {
                                node: identity.clone(),
                                state: results.get(&query.execution_id).map_or(ExecutionState::Unknown, |result| ExecutionState::Completed(ExecutionResult::Clone(result.clone()))),
                            },
                        })),
                        ControllerToNodeMessage::CloneRepository(command) if self.completes.load(Ordering::SeqCst) => {
                            let result = CloneExecutionResult::CloneReady(CloneReady {
                                node: identity.clone(),
                                spec: command.payload.spec.clone(),
                                repository_id: RepositoryId::new("repository"),
                                path: NodePath::new("/var/lib/ora/repositories/repo"),
                                commit: CommitId::new(COMMIT),
                            });
                            results.insert(command.execution_id.clone(), result.clone());
                            sequence += 1;
                            Some(NodeToControllerMessage::CloneResult(CloneResultMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                request_id: None,
                                operation_id: command.operation_id,
                                execution_id: command.execution_id,
                                sequence: Sequence::new(sequence),
                                payload: result,
                            }))
                        }
                        _ => None,
                    }
                }
                _ = beat.tick() => Some(NodeToControllerMessage::Heartbeat(HeartbeatMessage {
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

/// Everything one scenario runs against.
struct World {
    cloud: WorkspaceCloud,
    node: Node,
    timeline: Timeline,
    controller: Controller,
}

/// The running Controller process of a scenario, which a test may restart.
#[derive(Clone)]
struct Controller {
    config: RuntimeConfig,
    running: Arc<tokio::sync::Mutex<Option<Running>>>,
}

/// One started Controller runtime and the way to stop it.
struct Running {
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Controller {
    /// Opens and runs a new Controller runtime from the scenario's configuration.
    async fn start(&self) {
        let runtime = ControllerRuntime::<CloudStore>::open(self.config.clone()).unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            runtime
                .run(async {
                    let _ = stopped.await;
                })
                .await
        });
        *self.running.lock().await = Some(Running { stop, task });
    }

    /// Stops the running Controller and waits until it exited.
    async fn stop(&self) {
        let Running { stop, task } = self.running.lock().await.take().unwrap();
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
}

impl World {
    /// Stops the Controller and starts a new process with the same configuration and no memory of
    /// the old one's sessions; returns how many events the timeline held in between.
    async fn restart(&self) -> usize {
        self.controller.stop().await;
        let before = self.timeline.events().len();
        self.controller.start().await;
        before
    }
}

/// Runs `test` against a Controller with a Substrate and no static Node.
fn scenario<Fut: Future<Output = ()>>(requested_ref: &str, test: impl FnOnce(World) -> Fut) {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let home = root.path().join("controller");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let timeline = Timeline::default();
                let cloud = WorkspaceCloud::new(timeline.clone(), requested_ref);
                let served = cloud.serve().await;
                let substrate = Substrate {
                    journal: Arc::default(),
                    timeline: timeline.clone(),
                };
                let app = Router::new()
                    .route("/effects/{id}", any(effects))
                    .with_state(substrate);
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let effects_url = format!("http://{}", listener.local_addr().unwrap());
                tokio::spawn(async move { axum::serve(listener, app).await });
                let node = Node {
                    completes: Arc::new(AtomicBool::new(true)),
                    timeline: timeline.clone(),
                };
                let router_url = node.clone().serve().await;
                let config = RuntimeConfig {
                    home_directory: home,
                    persistence: Persistence::Cloud {
                        endpoint: served.endpoint.clone(),
                        claim_interval_ms: 100,
                        substrate: Some(SubstrateConfig {
                            effects_url,
                            router_url,
                            atespace: "local".into(),
                            request_timeout_ms: 2_000,
                        }),
                    },
                    protected_state_directories: Vec::new(),
                    controller_id: ControllerId::new("owner"),
                    nodes: Vec::new(),
                    session: SessionConfig {
                        io_timeout_ms: 1_000,
                        query_interval_ms: 20,
                    },
                    reconnect_ms: 50,
                    timezone: "Asia/Shanghai".into(),
                };
                let controller = Controller {
                    config,
                    running: Arc::default(),
                };
                controller.start().await;
                test(World {
                    cloud,
                    node,
                    timeline,
                    controller: controller.clone(),
                })
                .await;
                controller.stop().await;
                drop(served);
            });
    });
}

/// Waits until the operation reaches `state`.
async fn settled(world: &World, operation: &str, state: proto::OperationState) {
    let cloud = world.cloud.clone();
    let operation = operation.to_owned();
    world
        .timeline
        .until(|_| cloud.operation(&operation).state() == state)
        .await;
}

fn position(events: &[Event], event: &Event) -> usize {
    events
        .iter()
        .position(|candidate| candidate == event)
        .unwrap_or_else(|| panic!("{event:?} missing from {events:#?}"))
}

/// Creating a Workspace creates its sandbox, registers the Node the handshake presented and clones
/// into it before admission opens; stopping it reports idle, stops the session before the sandbox
/// is terminated, and never reports that deliberate end as a lost connection.
#[test]
fn create_and_stop_drive_the_workspace_sandbox_and_its_node() {
    scenario("main", |world| async move {
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        let workspace = world.cloud.workspace();
        assert_eq!(
            (
                workspace.base_commit_id.as_deref(),
                workspace.admission_open
            ),
            (Some(COMMIT), true)
        );
        let events = world.timeline.events();
        let registered = position(
            &events,
            &Event::Registered {
                incarnation: "incarnation-1".into(),
            },
        );
        assert!(position(&events, &Event::Advanced { to: "node" }) < registered);
        assert!(registered < position(&events, &Event::Advanced { to: "clone" }));
        assert!(
            position(&events, &Event::Dispatched)
                < position(&events, &Event::Advanced { to: "done" })
        );

        let stop = world.cloud.queue(proto::OperationKind::Stop);
        settled(&world, &stop, proto::OperationState::Succeeded).await;
        let events = world.timeline.events();
        let terminate = Event::Substrate {
            method: "PUT",
            kind: "sandbox_terminate",
        };
        assert!(position(&events, &Event::Idle { idle: true }) < position(&events, &terminate));
        // Invariant 4: the session ended before the sandbox was asked to terminate.
        let ended = position(&events, &Event::SessionEnded);
        assert!(ended < position(&events, &terminate));
        assert!(
            !events[ended..].contains(&Event::Status {
                connection: "disconnected"
            }),
            "a deliberate stop must not be reported as a lost connection: {events:#?}"
        );
        assert_eq!(world.cloud.workspace().observed_state, "stopped");
    });
}

/// A ref the Node protocol refuses, such as `HEAD`, blocks the clone step without registering any
/// execution: retrying could never make it valid.
#[test]
fn an_unclonable_ref_blocks_the_clone_step_without_a_dispatch() {
    scenario("HEAD", |world| async move {
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Blocked).await;
        let events = world.timeline.events();
        assert!(events.contains(&Event::Deferred {
            reason: "external_failure"
        }));
        assert!(!events.contains(&Event::Dispatched), "{events:#?}");
        assert!(!world.cloud.workspace().admission_open);
    });
}

/// A dispatch to the Node without a terminal result makes quiesce report the Node busy, which fails
/// the stop in Cloud instead of terminating a sandbox with work in flight.
#[test]
fn quiesce_reports_busy_while_a_dispatch_has_no_result() {
    scenario("main", |world| async move {
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        world.node.completes.store(false, Ordering::SeqCst);
        world.cloud.inject_pending(&create, node_id().as_str());
        let stop = world.cloud.queue(proto::OperationKind::Stop);
        settled(&world, &stop, proto::OperationState::Failed).await;
        let events = world.timeline.events();
        assert!(events.contains(&Event::Idle { idle: false }));
        assert!(
            !events.contains(&Event::Substrate {
                method: "PUT",
                kind: "sandbox_terminate"
            }),
            "{events:#?}"
        );
    });
}

/// A restarted Controller reconnects to the Workspace's live sandbox from Cloud's list of live
/// sandboxes, before and without any operation of that Workspace being claimed, and reports the
/// reconnected Node to Cloud: Cloud does not have to wait for a new operation to see it again.
#[test]
fn a_restarted_controller_reconnects_live_sandboxes_without_an_operation() {
    scenario("main", |world| async move {
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        let before = world.restart().await;
        let registered = Event::Registered {
            incarnation: "incarnation-1".into(),
        };
        world
            .timeline
            .until(|events| events[before..].contains(&registered))
            .await;
        let events = world.timeline.events()[before..].to_vec();
        assert!(
            position(&events, &Event::Listed { sandboxes: 1 }) < position(&events, &registered)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Claimed { .. })),
            "the session must come back without a claimed operation: {events:#?}"
        );
    });
}

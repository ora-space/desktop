//! Controlled cross-process evidence: the production Rust ordinary worker, real Go gRPC/Store,
//! PostgreSQL, and a protocol-speaking Node double. This is not a deployment reproduction.
use super::*;
use pretty_assertions::assert_eq;
use std::time::Instant;

/// One Node retains one execution across rounds and delivers its original result on request.
#[derive(Clone)]
struct JointNode {
    identity: NodeRuntimeIdentity,
    commands: Arc<Mutex<Vec<CloneRepositoryMessage>>>,
    outcome: Arc<Mutex<Option<bool>>>,
    accepted: Arc<tokio::sync::Notify>,
    handshake: bool,
    unknown: bool,
}

impl JointNode {
    /// Accepts concurrent reconnects while preserving the original command outside each session.
    async fn serve(self) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let acceptor = WsAcceptor::new(listener, NODE_PATH);
        let url = format!("ws://{}{NODE_PATH}", acceptor.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok(pending) = acceptor.accept().await {
                let Ok((receiver, sender)) = acceptor.open(pending).await else {
                    continue;
                };
                self.accepted.notify_one();
                let node = self.clone();
                tokio::spawn(async move { node.session(receiver, sender).await });
            }
        });
        url
    }

    /// Keeps heartbeats and pending replies live until the test requests real result delivery.
    async fn session<R: FrameReceiver, W: FrameSender>(&self, mut receiver: R, mut sender: W) {
        let mut greeted = false;
        let mut delivered = false;
        let mut beat = tokio::time::interval(Duration::from_millis(50));
        loop {
            let reply = tokio::select! {
                frame = receiver.recv() => {
                    let Ok(Some(frame)) = frame else { return };
                    let Ok(message) = decode_controller_frame(&frame) else { return };
                    match message {
                        ControllerToNodeMessage::Hello(_) if self.handshake => {
                            greeted = true;
                            Some(NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                payload: HelloAccepted { selected_version: CURRENT_PROTOCOL_VERSION,
                                    node: self.identity.clone(), capabilities: vec![NodeCapability::RepositoryClone, NodeCapability::RuntimeControl],
                                    model_adapter: None, thread_previews: false },
                            }))
                        }
                        ControllerToNodeMessage::BindRuntime(binding) => Some(NodeToControllerMessage::RuntimeControlState(RuntimeControlState { binding, unfinished_execution_ids: Vec::new() })),
                        ControllerToNodeMessage::ControlledClone(ControlledClone { command, .. }) => {
                            self.commands.lock().unwrap().push(command);
                            None
                        }
                        ControllerToNodeMessage::GetExecutionStatus(query) => {
                            let command = self.commands.lock().unwrap().first().cloned();
                            let state = if let Some(command) = command.filter(|_| !self.unknown) {
                                if let Some(success) = *self.outcome.lock().unwrap() {
                                    ExecutionState::Completed(ExecutionResult::Clone(Box::new(self.result(&command, success))))
                                } else { ExecutionState::Running }
                            } else { ExecutionState::Unknown };
                            Some(NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION, operation_id: query.operation_id,
                                execution_id: query.execution_id,
                                payload: ExecutionStatus { stop_unconfirmed: Vec::new(), node: self.identity.clone(), state },
                            }))
                        }
                        _ => None,
                    }
                }
                _ = beat.tick(), if greeted => {
                    let command = self.commands.lock().unwrap().first().cloned();
                    let outcome = *self.outcome.lock().unwrap();
                    if let (false, Some(command), Some(success)) = (delivered, command, outcome) {
                        delivered = true;
                        Some(NodeToControllerMessage::CloneResult(CloneResultMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION, request_id: None,
                            operation_id: command.operation_id.clone(), execution_id: command.execution_id.clone(),
                            sequence: Sequence::new(1), payload: self.result(&command, success),
                        }))
                    } else {
                        Some(NodeToControllerMessage::Heartbeat(HeartbeatMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            payload: Heartbeat { node: self.identity.clone() },
                        }))
                    }
                }
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

    /// Echoes the actual frozen command so result takeover exercises real identity validation.
    fn result(&self, command: &CloneRepositoryMessage, success: bool) -> CloneExecutionResult {
        if success {
            CloneExecutionResult::CloneReady(CloneReady {
                node: self.identity.clone(),
                spec: command.payload.spec.clone(),
                repository_id: RepositoryId::new("joint-repository"),
                path: NodePath::new("/joint/checkout"),
                commit: command.payload.spec.base_commit.clone(),
            })
        } else {
            CloneExecutionResult::CloneFailed(CloneFailed {
                node: self.identity.clone(),
                spec: command.payload.spec.clone(),
                failure: CloneFailureCode::SourceUnavailable,
                residual: CloneResidual::NoDirectory {},
            })
        }
    }
}

/// A fixture endpoint stages only initial scene and durable age; production RPCs drive all states.
struct Fixture {
    client: reqwest::Client,
    endpoint: String,
}

impl Fixture {
    /// Sends a fixture operation and keeps any server error visible in the test failure.
    async fn post(&self, path: &str, body: Value) -> Value {
        let response = self
            .client
            .post(format!("{}{path}", self.endpoint))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "fixture {path}: {status} {body}");
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&body).unwrap()
        }
    }

    /// Reads durable state until a requested observable transition or the scenario deadline.
    async fn until(&self, operation: &str, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let row: Value = self
                .client
                .get(format!("{}/operation/{operation}", self.endpoint))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            if predicate(&row) {
                return row;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "operation did not settle: {row}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// Runs real short rounds and original-result delivery over Go gRPC and durable PostgreSQL.
#[test]
#[ignore = "requires explicitly started TestShortRoundsJointServer and ORA_SHORT_ROUNDS_JOINT_METADATA"]
fn production_worker_yields_and_delivers_original_late_results_to_persistent_go_cloud() {
    ora_logging::initialize_test_clock();
    let metadata: Value = serde_json::from_slice(
        &fs::read(std::env::var("ORA_SHORT_ROUNDS_JOINT_METADATA").unwrap()).unwrap(),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture {
                client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap(),
                endpoint: metadata["fixture"].as_str().unwrap().into(),
            };
            for (case, handshake, late_result) in [
                ("unavailable", false, None),
                ("quiesce", false, None),
                ("pending", true, None),
                ("late_success", true, Some(true)),
                ("late_failure", true, Some(false)),
                ("unknown_at_deadline", true, None),
            ] {
                if std::env::var("ORA_SHORT_ROUNDS_JOINT_CASE").is_ok_and(|selected| selected != case) {
                    continue;
                }
                let scene = fixture
                    .post(
                        "/scene",
                        json!({"step":if case == "quiesce" {"quiesce"} else if handshake {"clone"} else {"node"},"target":true}),
                    )
                    .await;
                let operation = scene["operation"].as_str().unwrap();
                let node = JointNode {
                    identity: NodeRuntimeIdentity {
                        node_id: NodeId::new(scene["node"].as_str().unwrap()),
                        incarnation_id: NodeIncarnationId::new("joint-incarnation"),
                    },
                    commands: Arc::default(),
                    outcome: Arc::default(),
                    accepted: Arc::default(),
                    handshake,
                    unknown: case == "unknown_at_deadline",
                };
                let router_url = node.clone().serve().await;
                let controller = Controller {
                    config: RuntimeConfig {
                        management_tls: None,
                        home_directory: root.path().join(case),
                        persistence: Persistence::Cloud {
                            endpoint: metadata["grpc"].as_str().unwrap().into(),
                            claim_interval_ms: 25,
                            substrate: Some(SubstrateConfig {
                                effects_url: "http://127.0.0.1:1".into(),
                                router_url,
                                atespace: "joint".into(),
                                request_timeout_ms: 500,
                            }),
                        },
                        protected_state_directories: Vec::new(),
                        controller_id: ControllerId::new("short-rounds-joint"),
                        nodes: Vec::new(),
                        session: SessionConfig {
                            io_timeout_ms: 500,
                            query_interval_ms: 50,
                        },
                        reconnect_ms: 50,
                        timezone: "Asia/Shanghai".into(),
                    },
                    running: Arc::default(),
                };
                let started = Instant::now();
                controller.start().await;
                tokio::time::timeout(Duration::from_secs(5), node.accepted.notified())
                    .await
                    .unwrap();
                fixture.until(operation, |row| row["state"] == "running").await;
                // B is created while A owns the ordinary worker. It has an immediately runnable stop.
                let other = fixture
                    .post("/scene", json!({"step":"quiesce","target":false}))
                    .await;
                let deferred = fixture
                    .until(operation, |row| row["state"] == "retry_wait")
                    .await;
                let elapsed = started.elapsed();
                let expected = if handshake {
                    "clone_result_pending"
                } else {
                    "node_unavailable"
                };
                assert_eq!(deferred["reason"], expected);
                assert!(
                    elapsed < Duration::from_secs(if handshake { 7 } else { 4 }),
                    "{case} occupied worker for {elapsed:?}"
                );
                assert_eq!(deferred["attempts"], if handshake { 0 } else { 1 });
                if case == "quiesce" {
                    assert_eq!((deferred["idle_reports"].clone(), deferred["terminated"].clone()), (json!(0), json!(0)));
                }
                let remaining = deferred["retry_remaining"].as_f64().unwrap();
                assert!(
                    remaining > 0.0 && remaining <= if handshake { 1.1 } else { 5.1 },
                    "retry interval {remaining}"
                );
                let b = fixture
                    .until(other["operation"].as_str().unwrap(), |row| {
                        row["state"] == "succeeded"
                    })
                    .await;
                println!(
                    "SHORT_ROUNDS_JOINT {case} A={} yield_ms={} reason={} retry_remaining={} B={} B_state={}",
                    operation,
                    elapsed.as_millis(),
                    expected,
                    remaining,
                    b["id"],
                    b["state"]
                );
                if handshake {
                    let original = deferred["executions"].as_array().unwrap().clone();
                    assert_eq!(original.len(), 1);
                    let after = fixture
                        .until(operation, |row| {
                            row["version"].as_i64().unwrap()
                                >= deferred["version"].as_i64().unwrap() + 2
                                && row["state"] == "retry_wait"
                        })
                        .await;
                    assert_eq!(after["executions"], Value::Array(original.clone()));
                    if case != "unknown_at_deadline" {
                        assert_eq!(node.commands.lock().unwrap().len(), 1);
                    }
                    if case == "unknown_at_deadline" {
                        fixture.post(&format!("/expire/{operation}"), Value::Null).await;
                        let unknown = fixture.until(operation, |row| row["state"] == "blocked").await;
                        assert_eq!(unknown["reason"], "clone_result_unknown");
                        assert_eq!(unknown["executions"][0]["id"], original[0]["id"]);
                        assert_eq!(unknown["executions"].as_array().unwrap().len(), 1);
                        println!("SHORT_ROUNDS_JOINT {case} original_execution={} blocked_reason={}", original[0]["id"], unknown["reason"]);
                    }
                    if let Some(success) = late_result {
                        fixture
                            .post(&format!("/expire/{operation}"), Value::Null)
                            .await;
                        let timeout = fixture
                            .until(operation, |row| row["state"] == "blocked")
                            .await;
                        assert_eq!(timeout["reason"], "clone_result_timeout");
                        *node.outcome.lock().unwrap() = Some(success);
                        let recovered = fixture
                            .until(operation, |row| {
                                if success {
                                    row["state"] == "succeeded"
                                } else {
                                    row["reason"] == "clone_failed"
                                }
                            })
                            .await;
                        assert_eq!(recovered["executions"][0]["id"], original[0]["id"]);
                        assert_eq!(recovered["executions"].as_array().unwrap().len(), 1);
                        assert_eq!(node.commands.lock().unwrap().len(), 1);
                        println!(
                            "SHORT_ROUNDS_JOINT {case} original_execution={} timeout_then={} result={}",
                            original[0]["id"],
                            recovered["state"],
                            recovered["executions"][0]["result"]
                        );
                    }
                }
                controller.stop().await;
                fixture
                    .post(&format!("/retire/{operation}"), Value::Null)
                    .await;
            }
        });
}

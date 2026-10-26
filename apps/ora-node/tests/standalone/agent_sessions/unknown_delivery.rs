//! Unknown host identity fences the complete executable/IPC/SQLite/Git/HTTP delivery chain.
use super::*;
use ora_node_db::{DeliveryPlan, DeliveryProgress, FrozenOutcome, NodeDatabase, ProcessAttempt};
use ora_process_client::ProcessHost;
use ora_process_protocol::*;
use pretty_assertions::assert_eq;
use std::time::Duration;

#[derive(Clone, Copy)]
enum Window {
    BeforeBound,
    Expired,
}

const STOP_BOUND_MS: i64 = 3_600_000;

/// Reports the bounded fixture log when the real control connection closes unexpectedly.
async fn next(fixture: &Fixture, stream: &mut tokio::net::UnixStream) -> NodeToControllerMessage {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 90), async {
        loop {
            let Some(message) = read_node_message(stream).await.unwrap() else {
                let log = fs::read_to_string(fixture.path().join("agent-service.log"))
                    .unwrap_or_default();
                panic!(
                    "Node disconnected: {}",
                    log.lines().rev().take(10).collect::<Vec<_>>().join("\n")
                );
            };
            if !matches!(message, NodeToControllerMessage::Heartbeat(_)) {
                return message;
            }
        }
    })
    .await
    .unwrap()
}

/// The command wrapper sees both reads and mutations; no unknown checkout or identity may reach it.
fn assert_no_unknown_git(fixture: &Fixture, checkout: &std::path::Path) {
    let calls = fs::read_to_string(fixture.path().join("delivery-git-calls")).unwrap_or_default();
    assert!(
        !calls.contains(checkout.to_str().unwrap()),
        "unknown stop read HEAD or entered snapshot/push: {calls}"
    );
    assert!(
        !calls.contains("unknown-preparing") && !calls.contains("unknown-frozen"),
        "unknown stop entered delivery Git: {calls}"
    );
}

/// Independent fixture inspection deliberately bypasses the instrumented production Git binary.
fn refs(checkout: &std::path::Path) -> String {
    checkout_git(
        checkout,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )
}

/// Valid deliveries differ only in durable identities and the session/checkout they name.
fn input(
    server: &GitServerFixture,
    base: &CommitId,
    name: &str,
    session: &str,
    checkout: &str,
    branch: &str,
) -> DeliverRevisionMessage {
    let mut command = delivery::deliver(base, server);
    command.operation_id = OperationId::new(format!("op-{name}"));
    command.execution_id = ExecutionId::new(name);
    let spec = &mut command.payload.spec;
    spec.session_execution_id = ExecutionId::new(session);
    spec.checkout_execution_id = ExecutionId::new(checkout);
    spec.workspace_ref = WorkspaceRef::of(super::super::https::WORKSPACE, branch).unwrap();
    spec.revision_ref = RevisionRef::new(format!("refs/ora/revisions/{name}"));
    spec.delivery_ref = DeliveryRef::parse(&format!("refs/ora/deliveries/{name}")).unwrap();
    spec.history_key = ObjectKey::new(format!("runs/{name}/history.jsonl"));
    command
}

/// An actual completed delivery is the executor barrier; no absence assertion relies on a sleep.
async fn complete_control(
    fixture: &Fixture,
    unknown_checkout: &std::path::Path,
    stream: &mut tokio::net::UnixStream,
    command: &DeliverRevisionMessage,
    store: &str,
) -> RevisionDelivered {
    send(
        stream,
        ControllerToNodeMessage::DeliverRevision(command.clone()),
    )
    .await;
    loop {
        let message = next(fixture, stream).await;
        assert_no_unknown_git(fixture, unknown_checkout);
        match message {
            NodeToControllerMessage::UploadGrantNeeded(request) => {
                assert_eq!(
                    request.execution_id, command.execution_id,
                    "unknown stop requested upload"
                );
                send(
                    stream,
                    ControllerToNodeMessage::UploadGrant(UploadGrantMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: command.operation_id.clone(),
                        execution_id: command.execution_id.clone(),
                        payload: UploadGrant {
                            grants: vec![ObjectUploadGrant {
                                object_key: command.payload.spec.history_key.clone(),
                                url: PresignedUrl::new(format!(
                                    "http://{store}/{}",
                                    command.payload.spec.history_key.as_str()
                                )),
                                method: UploadMethod::Put,
                                headers: Default::default(),
                                expires_at: ora_logging::clock::now_local()
                                    + Duration::from_secs(/*secs*/ 3600),
                            }],
                        },
                    }),
                )
                .await;
            }
            NodeToControllerMessage::RevisionResult(result) => {
                assert_eq!(
                    result.execution_id, command.execution_id,
                    "unknown stop settled delivery"
                );
                send(
                    stream,
                    ControllerToNodeMessage::EventAck(EventAckMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: command.operation_id.clone(),
                        execution_id: command.execution_id.clone(),
                        sequence: result.sequence,
                        payload: EventAck {
                            node_id: NodeId::new("test-node"),
                        },
                    }),
                )
                .await;
                let RevisionExecutionResult::RevisionDelivered(delivered) = result.payload else {
                    panic!("positive delivery did not traverse snapshot/push/upload: {result:?}");
                };
                return delivered;
            }
            NodeToControllerMessage::ExecutionStatus(_) => {}
            NodeToControllerMessage::AgentSessionEnded(ended) => {
                assert_ne!(
                    ended.execution_id.as_str(),
                    EXECUTION,
                    "unknown stop emitted a terminal"
                );
            }
            other => panic!("unexpected delivery response: {other:?}"),
        }
    }
}

/// Preparing and frozen deliveries retain their original responsibility across bound expiry/reopen.
/// specs/test-cases/node/revision/restore-deadline-and-stop-confirmation.md#stop-confirmation-also-accepts-a-host-authoritative-notstarted
#[test]
fn unknown_identity_blocks_complete_delivery_before_and_after_bound_and_restart() {
    ora_logging::with_trace_logging(|| {
        for window in [Window::BeforeBound, Window::Expired] {
            let fixture = Fixture::new();
            let server =
                GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
            let config = configuration(&fixture, &server);
            prepare(&fixture, &config, &server);
            let original_checkout = checkout(&fixture);
            let original_path = PathBuf::from(original_checkout.path.as_str());
            fs::write(
                original_path.join("partial.txt"),
                "unconfirmed restore must remain private\n",
            )
            .unwrap();
            let original_refs = refs(&original_path);
            let original_head = fs::read(original_path.join(".git").join("HEAD")).unwrap();
            let mut node =
                Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
            node.configure_clone(config.clone()).unwrap();
            node.configure_git_access(server.access());
            let control_clone = request(&server, "control", "control");
            let result = node.submit_clone(control_clone.clone()).unwrap();
            let Some(CloneExecutionResult::CloneReady(control_checkout)) =
                clone_result(&result.state)
            else {
                panic!("control checkout missing: {result:?}");
            };
            let control_checkout = control_checkout.clone();
            drop(node);

            let database_path = fixture.config().home_directory.join("ora-node.sqlite3");
            let mut db = NodeDatabase::open(&database_path, fixture.config().identity).unwrap();
            db.bind_controller(&ControllerId::new("owner")).unwrap();
            let mut original = start("restore fixture");
            original.payload.spec.prior_revision = Some(PriorRevision {
                revision_id: RevisionId::new("prior"),
                final_commit: original_checkout.commit.clone(),
                base_commit: original_checkout.commit.clone(),
                git_repository_url: server.url.clone(),
                delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior").unwrap(),
            });
            db.accept_session(&original).unwrap();
            db.start_session(&original, &NodeIncarnationId::new("original"))
                .unwrap();
            let journal = db.session_journal().unwrap();
            let attempt = ProcessAttempt {
                execution: original.execution_id.clone(),
                host_directory: fixture.process().host_directory,
                expected_uid: fixture.process().expected_uid,
                intent: HostRunIntent {
                    scope: ScopeId::new(),
                    run: RunId::new(),
                    spec: RunSpec::new(
                        "/usr/bin/git",
                        &original_path,
                        DescendantPolicy::WaitForAll,
                    ),
                    host_disconnect: GuardianHostDisconnect::KeepRunning,
                },
            };
            journal
                .record_restore_attempt(&attempt, StopContainment::BestEffort)
                .unwrap();
            let now = (ora_logging::clock::now_local().unix_timestamp_nanos() / 1_000_000) as i64;
            let first = match window {
                Window::BeforeBound => now,
                Window::Expired => now - 2 * STOP_BOUND_MS,
            };
            journal
                .request_restore_stop(attempt.intent.run, first, STOP_BOUND_MS)
                .unwrap();
            journal
                .defer_restore_end(
                    &original.execution_id,
                    &AgentSessionEnded {
                        node: NodeRuntimeIdentity {
                            node_id: NodeId::new("test-node"),
                            incarnation_id: NodeIncarnationId::new("original"),
                        },
                        reason: AgentSessionEndReason::AgentFailed,
                        detail: Some("prior_revision_unavailable".into()),
                        scheduled_result: None,
                        artifact_manifest: None,
                    },
                )
                .unwrap();
            let mut control = start("completed control fixture");
            control.operation_id = OperationId::new("control-session-op");
            control.execution_id = ExecutionId::new("control-session");
            control.payload.spec.checkout_execution_id = control_clone.execution_id.clone();
            db.accept_session(&control).unwrap();
            db.start_session(&control, &NodeIncarnationId::new("original"))
                .unwrap();
            journal
                .end_session(
                    &control.execution_id,
                    AgentSessionEnded {
                        node: NodeRuntimeIdentity {
                            node_id: NodeId::new("test-node"),
                            incarnation_id: NodeIncarnationId::new("original"),
                        },
                        reason: AgentSessionEndReason::UserEnded,
                        detail: None,
                        scheduled_result: None,
                        artifact_manifest: None,
                    },
                )
                .unwrap();
            for session in [&original.execution_id, &control.execution_id] {
                let path = ora_history::history_path(
                    &fixture.config().home_directory.join("sessions"),
                    session.as_str(),
                )
                .unwrap();
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "").unwrap();
            }
            let pending = input(
                &server,
                &original_checkout.commit,
                "unknown-preparing",
                EXECUTION,
                "clone-exec-agent",
                "main",
            );
            let frozen = input(
                &server,
                &original_checkout.commit,
                "unknown-frozen",
                EXECUTION,
                "clone-exec-agent",
                "main",
            );
            db.accept_delivery(&frozen).unwrap();
            let frozen_directory =
                ora_utils::hash::sha256_hex(frozen.execution_id.as_str().as_bytes());
            let frozen_path = fixture
                .config()
                .home_directory
                .join("revision-deliveries")
                .join(&frozen_directory);
            fs::create_dir_all(&frozen_path).unwrap();
            fs::write(frozen_path.join("history.jsonl"), "").unwrap();
            let plan = DeliveryPlan {
                histories: Vec::new(),
                directory: frozen_directory,
                outcome: FrozenOutcome::Unchanged(RevisionUnchanged {
                    node: NodeRuntimeIdentity {
                        node_id: NodeId::new("test-node"),
                        incarnation_id: NodeIncarnationId::new("original"),
                    },
                    final_commit: original_checkout.commit.clone(),
                    base_commit: original_checkout.commit.clone(),
                    revision_ref: frozen.payload.spec.revision_ref.clone(),
                    history: StoredObject {
                        key: frozen.payload.spec.history_key.clone(),
                        size: 0,
                        sha256: Sha256Digest::new(ora_utils::hash::sha256_hex(b"")),
                    },
                }),
            };
            db.freeze_delivery(&frozen.execution_id, &plan).unwrap();
            drop(journal);
            drop(db);

            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            // The host owns the original Closed Scope but has never recorded this Run. No stub
            // translates "unknown" into "not started" and no SQL write manufactures a stop fact.
            runtime.block_on(async {
                let host = ProcessHost::new(attempt.host_directory.clone(), attempt.expected_uid);
                assert!(matches!(host.execute(HostOperation::CreateScope { scope: attempt.intent.scope }).await.unwrap(), HostReply::Scope(_)));
                tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
                    loop {
                        if matches!(host.execute(HostOperation::Close { scope: attempt.intent.scope }).await.unwrap(), HostReply::Scope(view) if matches!(view.last_observed, Some(ScopeState::Closed(_)))) { break; }
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                assert_eq!(host.execute(HostOperation::QueryRun { run: attempt.intent.run }).await.unwrap(), HostReply::Rejected(HostRejection::UnknownIdentity));
            });
            fs::remove_file(fixture.path().join("git")).unwrap();
            fs::write(fixture.path().join("git"), "#!/bin/sh\nprintf '%s\\t%s\\n' \"$PWD\" \"$*\" >> \"${0%/*}/delivery-git-calls\"\nexec /usr/bin/git \"$@\"\n").unwrap();
            fs::set_permissions(
                fixture.path().join("git"),
                fs::Permissions::from_mode(/*mode*/ 0o700),
            )
            .unwrap();
            let (store, objects) = runtime.block_on(delivery::object_store());
            for restart in 0..2 {
                fs::write(
                    PathBuf::from(control_checkout.path.as_str()).join("control.txt"),
                    format!("control round {restart}\n"),
                )
                .unwrap();
                let mut child = launch(&fixture, &config);
                runtime.block_on(async {
                    let (mut stream, _) = connect(&fixture).await;
                    // A status request takes the production explicit handover path to QueryRun.
                    send(&mut stream, query()).await;
                    loop {
                        if let NodeToControllerMessage::ExecutionStatus(status) =
                            next(&fixture, &mut stream).await
                            && status.execution_id == original.execution_id
                        {
                            assert_eq!(status.payload.state, ExecutionState::Running);
                            break;
                        }
                    }
                    for command in [&pending, &frozen] {
                        send(
                            &mut stream,
                            ControllerToNodeMessage::DeliverRevision(command.clone()),
                        )
                        .await;
                    }
                    let positive = input(
                        &server,
                        &control_checkout.commit,
                        &format!("control-{restart}"),
                        "control-session",
                        "clone-exec-control",
                        "control",
                    );
                    let delivered =
                        complete_control(&fixture, &original_path, &mut stream, &positive, &store)
                            .await;
                    assert_ne!(
                        delivered.final_commit, control_checkout.commit,
                        "positive control did not snapshot dirty checkout"
                    );
                    assert_eq!(
                        checkout_git(
                            &server.repository,
                            &["rev-parse", positive.payload.spec.delivery_ref.as_str()]
                        ),
                        delivered.final_commit.as_str()
                    );
                    for command in [&pending, &frozen] {
                        send(
                            &mut stream,
                            ControllerToNodeMessage::GetExecutionStatus(
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
                        .await;
                        loop {
                            if let NodeToControllerMessage::ExecutionStatus(status) =
                                next(&fixture, &mut stream).await
                                && status.execution_id == command.execution_id
                            {
                                assert_eq!(status.payload.state, ExecutionState::Running);
                                break;
                            }
                        }
                    }
                });
                child.terminate();
                let calls = fs::read_to_string(fixture.path().join("delivery-git-calls")).unwrap();
                assert!(
                    calls.contains(control_checkout.path.as_str()),
                    "positive control did not reach instrumented Git"
                );
                assert!(
                    !calls.contains(original_path.to_str().unwrap()),
                    "unknown stop read HEAD or entered snapshot/push: {calls}"
                );
                assert!(
                    !calls.contains("unknown-preparing") && !calls.contains("unknown-frozen"),
                    "unknown stop entered delivery Git: {calls}"
                );
                assert!(
                    !server.server.served().iter().any(|request| request
                        .execution_id
                        .as_deref()
                        .is_some_and(|id| id.starts_with("unknown-"))),
                    "unknown stop reached Git Server"
                );
                let stored = objects.lock().unwrap();
                assert_eq!(stored.len(), restart + 1);
                assert!(!stored.contains_key(pending.payload.spec.history_key.as_str()));
                assert!(!stored.contains_key(frozen.payload.spec.history_key.as_str()));
                drop(stored);
                assert_eq!(refs(&original_path), original_refs);
                assert_eq!(
                    fs::read(original_path.join(".git").join("HEAD")).unwrap(),
                    original_head
                );
                assert_eq!(
                    fs::read_to_string(original_path.join("partial.txt")).unwrap(),
                    "unconfirmed restore must remain private\n"
                );
                assert_eq!(
                    refs(&server.repository)
                        .lines()
                        .filter(|line| line.contains("unknown-"))
                        .count(),
                    0
                );
                assert!(frozen_path.join("history.jsonl").is_file());
                let db = NodeDatabase::open(&database_path, fixture.config().identity).unwrap();
                let journal = db.session_journal().unwrap();
                let retained = journal
                    .pending_restore_attempts(&original.execution_id)
                    .unwrap();
                assert_eq!(retained.len(), 1);
                assert_eq!(retained[0].process, attempt);
                assert_eq!(retained[0].first_stop_at_ms, Some(first));
                assert_eq!(retained[0].stop_bound_at_ms, Some(first + STOP_BOUND_MS));
                assert_eq!(
                    retained[0].stop_unconfirmed,
                    matches!(window, Window::Expired)
                );
                assert_eq!(
                    db.find_delivery(&pending.operation_id, &pending.execution_id)
                        .unwrap()
                        .unwrap()
                        .progress,
                    DeliveryProgress::Preparing
                );
                assert_eq!(
                    db.find_delivery(&frozen.operation_id, &frozen.execution_id)
                        .unwrap()
                        .unwrap()
                        .progress,
                    DeliveryProgress::Frozen(plan.clone())
                );
                assert!(!db.pending_events().unwrap().iter().any(|event| matches!(event, NodeToControllerMessage::AgentSessionEnded(ended) if ended.execution_id == original.execution_id)));
            }
        }
    });
}

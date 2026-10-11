//! Sessions resuming a prior Revision through the real Cloud adapter and WebSocket transport: the
//! capability gate on registration and the download-grant relay against the fake Cloud.
use super::*;
use pretty_assertions::assert_eq;
use workspace_cloud::{PRIOR_BUNDLE, PRIOR_FINAL, SIGNATURE, STORE_HOST};

#[derive(Default)]
struct RestoreData {
    /// Unsequenced grant requests, each sent once.
    needs: Vec<DownloadGrantNeededMessage>,
    /// Every answer received, kept only so tests can compare it with what Cloud issued.
    answers: Vec<DownloadGrantMessage>,
}

/// The restore half of the fake sandbox Node.
#[derive(Clone, Default)]
pub(super) struct RestoreNode {
    data: Arc<Mutex<RestoreData>>,
    pub(super) enabled: Arc<AtomicBool>,
}

impl RestoreNode {
    /// Sends the next pending grant request.
    pub(super) fn next(&self) -> Option<NodeToControllerMessage> {
        self.data
            .lock()
            .unwrap()
            .needs
            .pop()
            .map(NodeToControllerMessage::DownloadGrantNeeded)
    }

    /// Records a received answer frame.
    pub(super) fn answer(&self, message: DownloadGrantMessage, timeline: &Timeline) {
        message.validate().unwrap();
        let event = Event::DownloadReceived {
            execution: message.execution_id.as_str().into(),
            granted: matches!(message.payload, DownloadGrant::Granted { .. }),
        };
        self.data.lock().unwrap().answers.push(message);
        timeline.push(event);
    }

    /// Reports that the session holds no valid grant for its prior bundle.
    fn need(&self, start: &StartAgentSessionMessage, node: NodeId) {
        self.data
            .lock()
            .unwrap()
            .needs
            .push(DownloadGrantNeededMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id.clone(),
                execution_id: start.execution_id.clone(),
                payload: DownloadGrantNeeded { node_id: node },
            });
    }

    /// Every answer received so far, in order.
    fn answers(&self) -> Vec<DownloadGrantMessage> {
        self.data.lock().unwrap().answers.clone()
    }
}

/// Starts a Workspace whose Node runs sessions and restores prior Revisions.
async fn workspace(world: &World) {
    world.node.agents.enabled.store(true, Ordering::SeqCst);
    world.node.restores.enabled.store(true, Ordering::SeqCst);
    let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
    settled(world, &create, proto::OperationState::Succeeded).await;
}

/// Queues a resumed session for `run` and waits until the Node received its controlled start.
async fn resuming(world: &World, run: &str) -> StartAgentSessionMessage {
    world.cloud.queue_resumed_agent(run);
    world
        .timeline
        .until(|events| {
            world.cloud.agent_records().iter().any(|r| {
                r.operation_id == run
                    && events.contains(&Event::AgentStarted {
                        execution: r.execution_id.clone(),
                    })
            })
        })
        .await;
    let record = world
        .cloud
        .agent_records()
        .into_iter()
        .find(|r| r.operation_id == run)
        .unwrap();
    world
        .node
        .agents
        .started(&ExecutionId::new(record.execution_id))
}

/// Counts the answers the Node received for `execution`, granted or refused.
fn received(events: &[Event], execution: &ExecutionId, granted: bool) -> usize {
    let wanted = Event::DownloadReceived {
        execution: execution.as_str().into(),
        granted,
    };
    events.iter().filter(|e| **e == wanted).count()
}

/// What the Node must receive for one Cloud response: the grant with headers and expiry as issued.
fn relayed(
    response: &proto::GrantRevisionDownloadResponse,
    start: &StartAgentSessionMessage,
) -> DownloadGrantMessage {
    DownloadGrantMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start.operation_id.clone(),
        execution_id: start.execution_id.clone(),
        payload: DownloadGrant::Granted {
            grants: response
                .grants
                .iter()
                .map(|grant| ObjectDownloadGrant {
                    object_key: ObjectKey::new(grant.object_key.clone()),
                    url: PresignedUrl::new(grant.url.clone()),
                    method: DownloadMethod::Get,
                    headers: grant.headers.clone().into_iter().collect(),
                    expires_at: time::OffsetDateTime::from_unix_timestamp(
                        grant.expires_at.unwrap().seconds,
                    )
                    .unwrap(),
                })
                .collect(),
        },
    }
}

/// A session that resumes a prior Revision stays queued in Cloud while its Node cannot restore,
/// and reaches the Node with the prior Revision once a restore-capable handshake happened.
#[test]
fn resumed_sessions_wait_for_a_restore_capable_handshake() {
    scenario("main", |world| async move {
        world.node.agents.enabled.store(true, Ordering::SeqCst);
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        world.cloud.queue_resumed_agent("run");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            world.cloud.agent_records().is_empty(),
            "a Node that cannot restore must not receive a resumed session"
        );
        world.node.restores.enabled.store(true, Ordering::SeqCst);
        world.restart().await;
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::AgentStarted { .. }))
            })
            .await;
        let records = world.cloud.agent_records();
        assert_eq!(records.len(), 1);
        let start = world
            .node
            .agents
            .started(&ExecutionId::new(records[0].execution_id.clone()));
        assert_eq!(
            start.payload.spec.prior_revision,
            Some(PriorRevision {
                revision_id: RevisionId::new("revision-1"),
                final_commit: CommitId::new(PRIOR_FINAL),
                bundle: StoredObject {
                    key: ObjectKey::new(PRIOR_BUNDLE),
                    size: 42,
                    sha256: Sha256Digest::new(
                        "3333333333333333333333333333333333333333333333333333333333333333"
                    ),
                },
            })
        );
    });
}

/// Neither negotiated capability can substitute for the other when a session needs both snapshots.
#[test]
fn model_backed_resumes_require_both_capabilities_and_preserve_both_inputs() {
    for (restore_enabled, model_enabled) in [(false, true), (true, false), (false, false)] {
        scenario("main", |world| async move {
            world.node.agents.enabled.store(true, Ordering::SeqCst);
            world
                .node
                .restores
                .enabled
                .store(restore_enabled, Ordering::SeqCst);
            world
                .node
                .agents
                .model_proxy
                .store(model_enabled, Ordering::SeqCst);
            let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
            settled(&world, &create, proto::OperationState::Succeeded).await;
            world.cloud.queue_session_with_model(
                "run",
                Some(workspace_cloud::WorkspaceCloud::prior_revision()),
                "binding-1",
            );
            tokio::time::sleep(Duration::from_millis(/*millis*/ 300)).await;
            assert_eq!(world.cloud.agent_records(), Vec::new());
            world.node.restores.enabled.store(true, Ordering::SeqCst);
            world.node.agents.model_proxy.store(true, Ordering::SeqCst);
            world.restart().await;
            world
                .timeline
                .until(|events| {
                    events
                        .iter()
                        .any(|e| matches!(e, Event::AgentStarted { .. }))
                })
                .await;
            let records = world.cloud.agent_records();
            assert_eq!(records.len(), 1);
            let start = world
                .node
                .agents
                .started(&ExecutionId::new(records[0].execution_id.clone()));
            let spec = start.payload.spec;
            assert_eq!(
                spec.model_binding_id,
                Some(ModelBindingId::new("binding-1"))
            );
            assert_eq!(
                spec.prior_revision,
                Some(PriorRevision {
                    revision_id: RevisionId::new("revision-1"),
                    final_commit: CommitId::new(PRIOR_FINAL),
                    bundle: StoredObject {
                        key: ObjectKey::new(PRIOR_BUNDLE),
                        size: 42,
                        sha256: Sha256Digest::new(
                            "3333333333333333333333333333333333333333333333333333333333333333"
                        ),
                    },
                })
            );
        });
    }
}

/// A download grant is requested only when the Node asks, reaches it verbatim, and a Cloud
/// refusal is answered as refused; no log line ever carries the grant URL or signature.
#[test]
fn download_grants_are_relayed_unlogged_and_refusals_answered() {
    let recorder = deliveries::Recorder::default();
    ora_logging::with_recorded_trace_logging(recorder.clone(), || {
        run_scenario("main", |world| async move {
            workspace(&world).await;
            let start = resuming(&world, "run").await;
            let execution = start.execution_id.clone();
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(
                world.cloud.issued_downloads().is_empty(),
                "the Controller asks Cloud only when the Node does"
            );
            world.node.restores.need(&start, node_id());
            world
                .timeline
                .until(|events| received(events, &execution, /*granted*/ true) == 1)
                .await;
            let issued = world.cloud.issued_downloads();
            assert_eq!(
                issued
                    .iter()
                    .map(|(request, _)| request.execution_id.clone())
                    .collect::<Vec<_>>(),
                vec![execution.as_str().to_owned()]
            );
            assert_eq!(
                world.node.restores.answers(),
                vec![relayed(&issued[0].1, &start)]
            );

            let before = world.timeline.events().len();
            world.cloud.refuse_downloads(true);
            world.node.restores.need(&start, node_id());
            world
                .timeline
                .until(|events| received(events, &execution, /*granted*/ false) == 1)
                .await;
            assert_eq!(
                world.node.restores.answers()[1],
                DownloadGrantMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: start.operation_id.clone(),
                    execution_id: execution.clone(),
                    payload: DownloadGrant::Refused {},
                }
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(
                !world.timeline.events()[before..].contains(&Event::SessionEnded),
                "a refusal must not end the connection"
            );
        });
    });
    let lines = recorder.0.lock().unwrap().clone();
    assert!(
        lines.iter().any(|l| l.contains("download grant relayed")),
        "the relay must be observable without its content: {lines:#?}"
    );
    let leaked: Vec<_> = lines
        .iter()
        .filter(|l| l.contains(STORE_HOST) || l.contains(SIGNATURE))
        .collect();
    assert!(leaked.is_empty(), "grants leaked into logs: {leaked:#?}");
}

/// A Cloud outage is retried rather than refused, and a new connection asks Cloud again only once
/// the Node repeats its request: the Controller keeps no grant or request across connections.
#[test]
fn outages_are_retried_and_reconnects_wait_for_the_node_to_ask_again() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let start = resuming(&world, "run").await;
        let execution = start.execution_id.clone();
        world.cloud.fail_downloads(2);
        world.node.restores.need(&start, node_id());
        world
            .timeline
            .until(|events| received(events, &execution, /*granted*/ true) == 1)
            .await;
        assert_eq!(
            received(&world.timeline.events(), &execution, /*granted*/ false),
            0,
            "an outage must never reach the Node as a refusal"
        );

        let before = world.timeline.events().len();
        world.node.disconnect.store(true, Ordering::SeqCst);
        world
            .timeline
            .until(|events| {
                events[before..].contains(&Event::Status {
                    connection: "connected",
                })
            })
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(world.cloud.issued_downloads().len(), 1);
        world.node.restores.need(&start, node_id());
        world
            .timeline
            .until(|events| received(&events[before..], &execution, /*granted*/ true) == 1)
            .await;
        let issued = world.cloud.issued_downloads();
        assert_eq!(issued.len(), 2);
        assert_eq!(
            world.node.restores.answers(),
            issued
                .iter()
                .map(|(_, response)| relayed(response, &start))
                .collect::<Vec<_>>()
        );
    });
}

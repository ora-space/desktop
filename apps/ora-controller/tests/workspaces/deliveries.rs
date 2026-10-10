//! Revision delivery dispatch and upload-grant relay through the real Cloud adapter and WebSocket
//! transport, against the fake Cloud's delivery authority and a delivery-capable fake Node.
use super::*;
use pretty_assertions::assert_eq;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};
use workspace_cloud::{SIGNATURE, STORE_HOST};

const FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
const BUNDLE_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const HISTORY_DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[derive(Default)]
struct DeliveryData {
    starts: HashMap<ExecutionId, DeliverRevisionMessage>,
    /// Terminal events retained until acknowledged, replayed once per connection.
    outbox: BTreeMap<String, RevisionResultMessage>,
    /// Unsequenced grant requests, each sent once.
    needs: Vec<UploadGrantNeededMessage>,
    terminal: HashMap<ExecutionId, RevisionExecutionResult>,
    /// Grants received, kept only so tests can compare them with what Cloud issued.
    grants: Vec<UploadGrantMessage>,
}

/// The delivery half of the fake sandbox Node.
#[derive(Clone, Default)]
pub(super) struct DeliveryNode {
    data: Arc<Mutex<DeliveryData>>,
    pub(super) enabled: Arc<AtomicBool>,
}

impl DeliveryNode {
    /// Verifies the controlled envelope and keeps the first command of each execution.
    pub(super) fn start(&self, value: ControlledDeliverRevision, timeline: &Timeline) {
        value.validate().unwrap();
        let command = value.command;
        let mut data = self.data.lock().unwrap();
        if let Some(old) = data.starts.get(&command.execution_id) {
            assert_eq!(old, &command);
        }
        data.starts
            .insert(command.execution_id.clone(), command.clone());
        drop(data);
        timeline.push(Event::DeliveryStarted {
            execution: command.execution_id.as_str().into(),
        });
    }

    /// Running until completed; a known terminal is reported without manufacturing a receipt.
    pub(super) fn status(&self, execution: &ExecutionId) -> Option<ExecutionState> {
        let data = self.data.lock().unwrap();
        data.starts.get(execution).map(|_| {
            data.terminal
                .get(execution)
                .map_or(ExecutionState::Running, |result| {
                    ExecutionState::Completed(ExecutionResult::Revision(Box::new(result.clone())))
                })
        })
    }

    /// Sends pending grant requests first, then the retained terminal not yet sent on this
    /// connection.
    pub(super) fn next(&self, sent: &mut HashSet<String>) -> Option<NodeToControllerMessage> {
        let mut data = self.data.lock().unwrap();
        if let Some(need) = data.needs.pop() {
            return Some(NodeToControllerMessage::UploadGrantNeeded(need));
        }
        let (key, event) = data.outbox.iter().find(|(key, _)| !sent.contains(*key))?;
        sent.insert(key.clone());
        Some(NodeToControllerMessage::RevisionResult(event.clone()))
    }

    /// Records a received grant frame.
    pub(super) fn grant(&self, message: UploadGrantMessage, timeline: &Timeline) {
        message.validate().unwrap();
        let execution = message.execution_id.as_str().to_owned();
        self.data.lock().unwrap().grants.push(message);
        timeline.push(Event::GrantReceived { execution });
    }

    /// Drops the retained terminal only for an acknowledgement of exactly that event.
    pub(super) fn ack(&self, ack: &EventAckMessage, timeline: &Timeline) {
        let removed = self
            .data
            .lock()
            .unwrap()
            .outbox
            .remove(ack.execution_id.as_str())
            .is_some_and(|event| event.sequence == ack.sequence);
        if removed {
            timeline.push(Event::DeliveryAck {
                execution: ack.execution_id.as_str().into(),
                sequence: ack.sequence.value(),
            });
        }
    }

    /// Reports that the delivery holds no valid grant for its two frozen objects.
    fn need(&self, execution: &ExecutionId) {
        let mut data = self.data.lock().unwrap();
        let start = data.starts.get(execution).unwrap().clone();
        data.needs.push(UploadGrantNeededMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: start.operation_id.clone(),
            execution_id: execution.clone(),
            payload: UploadGrantNeeded {
                node_id: node_id(),
                checksums: checksums(&start),
            },
        });
    }

    /// Completes the delivery and retains its terminal event until acknowledged.
    fn complete(&self, execution: &ExecutionId) -> RevisionExecutionResult {
        let mut data = self.data.lock().unwrap();
        let start = data.starts.get(execution).unwrap().clone();
        let spec = &start.payload.spec;
        let result = RevisionExecutionResult::RevisionDelivered(RevisionDelivered {
            node: NodeRuntimeIdentity {
                node_id: node_id(),
                incarnation_id: NodeIncarnationId::new("incarnation-1"),
            },
            final_commit: CommitId::new(FINAL),
            base_commit: spec.base_commit.clone(),
            revision_ref: spec.revision_ref.clone(),
            bundle: StoredObject {
                key: spec.bundle_key.clone(),
                size: 10,
                sha256: Sha256Digest::new(BUNDLE_DIGEST),
            },
            history: StoredObject {
                key: spec.history_key.clone(),
                size: 20,
                sha256: Sha256Digest::new(HISTORY_DIGEST),
            },
        });
        data.terminal.insert(execution.clone(), result.clone());
        data.outbox.insert(
            execution.as_str().into(),
            RevisionResultMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id,
                execution_id: execution.clone(),
                sequence: Sequence::new(/*value*/ 1),
                payload: result.clone(),
            },
        );
        result
    }

    fn started(&self, execution: &ExecutionId) -> DeliverRevisionMessage {
        self.data.lock().unwrap().starts[execution].clone()
    }

    fn grants(&self) -> Vec<UploadGrantMessage> {
        self.data.lock().unwrap().grants.clone()
    }
}

/// The digests the fake Node froze for a delivery's two objects.
fn checksums(start: &DeliverRevisionMessage) -> BTreeMap<ObjectKey, Sha256Digest> {
    BTreeMap::from([
        (
            start.payload.spec.bundle_key.clone(),
            Sha256Digest::new(BUNDLE_DIGEST),
        ),
        (
            start.payload.spec.history_key.clone(),
            Sha256Digest::new(HISTORY_DIGEST),
        ),
    ])
}

/// The Cloud request map the Controller must send for those digests.
fn requested(run: &str) -> HashMap<String, String> {
    HashMap::from([
        (format!("runs/{run}/revision.bundle"), BUNDLE_DIGEST.into()),
        (format!("runs/{run}/history.jsonl"), HISTORY_DIGEST.into()),
    ])
}

/// What the Node must receive for one Cloud response: every grant with headers and expiry as issued.
fn relayed(
    response: &proto::GrantRevisionUploadResponse,
    start: &DeliverRevisionMessage,
) -> UploadGrantMessage {
    UploadGrantMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start.operation_id.clone(),
        execution_id: start.execution_id.clone(),
        payload: UploadGrant {
            grants: response
                .grants
                .iter()
                .map(|grant| ObjectUploadGrant {
                    object_key: ObjectKey::new(grant.object_key.clone()),
                    url: PresignedUrl::new(grant.url.clone()),
                    method: UploadMethod::Put,
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

/// Starts a Workspace whose Node advertises Revision delivery.
async fn workspace(world: &World) {
    world.node.deliveries.enabled.store(true, Ordering::SeqCst);
    let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
    settled(world, &create, proto::OperationState::Succeeded).await;
}

/// Queues a delivery for `run` and waits until the Node received its controlled envelope.
async fn delivering(world: &World, run: &str) -> ExecutionId {
    world.cloud.queue_delivery(run);
    world
        .timeline
        .until(|events| {
            world.cloud.delivery_records().iter().any(|r| {
                r.operation_id == run
                    && events.contains(&Event::DeliveryStarted {
                        execution: r.execution_id.clone(),
                    })
            })
        })
        .await;
    ExecutionId::new(record(world, run).execution_id)
}

fn record(world: &World, run: &str) -> proto::ExecutionRecord {
    world
        .cloud
        .delivery_records()
        .into_iter()
        .find(|r| r.operation_id == run)
        .unwrap()
}

/// Counts the grant frames the Node received for `execution`.
fn received(events: &[Event], execution: &ExecutionId) -> usize {
    events
        .iter()
        .filter(|e| {
            **e == Event::GrantReceived {
                execution: execution.as_str().into(),
            }
        })
        .count()
}

/// Records every logged event and span field, as text, under the test-scoped subscriber.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

/// Writes each visited field as `name=value`.
struct Line<'a>(&'a mut String);

impl Visit for Line<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

impl<S: tracing::Subscriber> Layer<S> for Recorder {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut line = format!("{} ", event.metadata().target());
        event.record(&mut Line(&mut line));
        self.0.lock().unwrap().push(line);
    }

    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        _: &tracing::span::Id,
        _: Context<'_, S>,
    ) {
        let mut line = format!("span {} ", attributes.metadata().name());
        attributes.record(&mut Line(&mut line));
        self.0.lock().unwrap().push(line);
    }
}

/// A registered delivery is sent only with a current execution permit; grants are requested when
/// the Node asks, bound to its digests, and reach it verbatim, while no log line ever carries a
/// grant URL or signature.
#[test]
fn delivery_waits_for_its_permit_and_relays_checksum_bound_grants_unlogged() {
    let recorder = Recorder::default();
    ora_logging::with_recorded_trace_logging(recorder.clone(), || {
        run_scenario("main", |world| async move {
            workspace(&world).await;
            world.cloud.close_agent_input(true);
            let input = world.cloud.queue_delivery("run");
            world
                .timeline
                .until(|events| events.contains(&Event::DeliveryRegistered { run: "run".into() }))
                .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(
                !world
                    .timeline
                    .events()
                    .iter()
                    .any(|e| matches!(e, Event::DeliveryStarted { .. })),
                "a closed runtime input cannot authorize a delivery"
            );
            world.cloud.close_agent_input(false);
            let record = record(&world, "run");
            let started = Event::DeliveryStarted {
                execution: record.execution_id.clone(),
            };
            world
                .timeline
                .until(|events| events.contains(&started))
                .await;
            let execution = ExecutionId::new(record.execution_id.clone());
            let Some(proto::execution_input::Spec::DeliverRevision(spec)) = input.spec.clone()
            else {
                unreachable!()
            };
            assert_eq!(record.input, Some(input));
            assert_eq!(
                world.node.deliveries.started(&execution),
                DeliverRevisionMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(record.node_operation_id.clone()),
                    execution_id: execution.clone(),
                    payload: DeliverRevision {
                        spec: DeliverRevisionSpec {
                            node_id: node_id(),
                            session_execution_id: ExecutionId::new("session-of-run"),
                            checkout_execution_id: ExecutionId::new(spec.checkout_execution_id),
                            base_commit: CommitId::new(COMMIT),
                            revision_ref: RevisionRef::new("refs/ora/revisions/run"),
                            bundle_key: ObjectKey::new("runs/run/revision.bundle"),
                            history_key: ObjectKey::new("runs/run/history.jsonl"),
                        },
                    },
                }
            );
            // Dispatch alone requests nothing: a grant before the Node froze its objects could
            // not bind their checksums.
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(world.cloud.issued_grants().is_empty());

            world.node.deliveries.need(&execution);
            world
                .timeline
                .until(|events| received(events, &execution) == 1)
                .await;
            let issued = world.cloud.issued_grants();
            assert_eq!(issued.len(), 1);
            let (request, response) = &issued[0];
            assert_eq!(
                (request.execution_id.as_str(), &request.checksums),
                (execution.as_str(), &requested("run"))
            );
            let start = world.node.deliveries.started(&execution);
            assert_eq!(
                world.node.deliveries.grants(),
                vec![relayed(response, &start)]
            );
        });
    });
    let lines = recorder.0.lock().unwrap().clone();
    assert!(
        lines.iter().any(|l| l.contains("upload grants relayed")),
        "the relay must be observable without its content: {lines:#?}"
    );
    let leaked: Vec<_> = lines
        .iter()
        .filter(|l| l.contains(STORE_HOST) || l.contains(SIGNATURE))
        .collect();
    assert!(leaked.is_empty(), "grants leaked into logs: {leaked:#?}");
}

/// A Node that does not advertise Revision delivery leaves the work item queued in Cloud.
#[test]
fn delivery_registration_waits_for_a_capable_handshake() {
    scenario("main", |world| async move {
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        world.cloud.queue_delivery("run");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(world.cloud.delivery_records().is_empty());
        world.node.deliveries.enabled.store(true, Ordering::SeqCst);
        world.restart().await;
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::DeliveryStarted { .. }))
            })
            .await;
        assert_eq!(world.cloud.delivery_records().len(), 1);
    });
}

/// A new connection to a Node still running the delivery refreshes its grants with the digests
/// the Node reported, without waiting for the Node to ask again.
#[test]
fn reconnect_refreshes_grants_for_an_unfinished_delivery() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = delivering(&world, "run").await;
        world.node.deliveries.need(&execution);
        world
            .timeline
            .until(|events| received(events, &execution) == 1)
            .await;
        let before = world.timeline.events().len();
        world.node.disconnect.store(true, Ordering::SeqCst);
        world
            .timeline
            .until(|events| received(&events[before..], &execution) == 1)
            .await;
        let events = world.timeline.events()[before..].to_vec();
        assert!(
            position(&events, &Event::SessionEnded)
                < position(
                    &events,
                    &Event::GrantReceived {
                        execution: execution.as_str().into()
                    }
                )
        );
        let issued = world.cloud.issued_grants();
        assert_eq!(
            issued
                .iter()
                .map(|(request, _)| request.checksums.clone())
                .collect::<Vec<_>>(),
            vec![requested("run"), requested("run")]
        );
        let start = world.node.deliveries.started(&execution);
        assert_eq!(
            world.node.deliveries.grants(),
            issued
                .iter()
                .map(|(_, response)| relayed(response, &start))
                .collect::<Vec<_>>()
        );
        assert_eq!(world.node.deliveries.data.lock().unwrap().starts.len(), 1);
    });
}

/// The terminal envelope is acknowledged only after Cloud committed it; Completed status replies
/// seen meanwhile settle nothing, and a slow commit does not stall the connection.
#[test]
fn revision_result_is_acknowledged_only_after_cloud_commits() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = delivering(&world, "run").await;
        let before = world.timeline.events().len();
        world.cloud.block_agent("run", true);
        let result = world.node.deliveries.complete(&execution);
        world
            .timeline
            .until(|events| events.contains(&Event::RevisionTakeoverAttempt { run: "run".into() }))
            .await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(record(&world, "run").result, None);
        assert_eq!(world.cloud.queried_revisions(), 0);
        assert!(
            !world.timeline.events()[before..]
                .iter()
                .any(|e| matches!(e, Event::DeliveryAck { .. } | Event::SessionEnded)),
            "unexpected early receipt or disconnect: {:#?}",
            world.timeline.events()
        );
        world.cloud.block_agent("run", false);
        let ack = Event::DeliveryAck {
            execution: execution.as_str().into(),
            sequence: 1,
        };
        world.timeline.until(|events| events.contains(&ack)).await;
        let events = world.timeline.events();
        assert!(
            position(&events, &Event::RevisionTaken { run: "run".into() })
                < position(&events, &ack)
        );
        let RevisionExecutionResult::RevisionDelivered(delivered) = result else {
            unreachable!()
        };
        assert_eq!(
            record(&world, "run").result,
            Some(proto::ExecutionResult {
                node: Some(proto::NodeIdentity {
                    node_id: node_id().as_str().into(),
                    node_incarnation_id: "incarnation-1".into(),
                }),
                outcome: Some(proto::execution_result::Outcome::RevisionDelivered(
                    proto::RevisionDelivered {
                        final_commit: FINAL.into(),
                        base_commit: COMMIT.into(),
                        revision_ref: "refs/ora/revisions/run".into(),
                        bundle: Some(proto::StoredObject {
                            key: delivered.bundle.key.as_str().into(),
                            size: 10,
                            sha256: BUNDLE_DIGEST.into(),
                        }),
                        history: Some(proto::StoredObject {
                            key: delivered.history.key.as_str().into(),
                            size: 20,
                            sha256: HISTORY_DIGEST.into(),
                        }),
                    }
                )),
            })
        );
    });
}

/// Stopping the Controller never fails a registered delivery; a new process resumes it through
/// status queries, waits for the Node's own grant request, and settles it from the real envelope.
#[test]
fn restart_resumes_a_registered_delivery_without_failing_it() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = delivering(&world, "run").await;
        world.node.deliveries.need(&execution);
        world
            .timeline
            .until(|events| received(events, &execution) == 1)
            .await;
        let before = world.restart().await;
        assert_eq!(record(&world, "run").result, None);
        world
            .timeline
            .until(|events| {
                events[before..].contains(&Event::Registered {
                    incarnation: "incarnation-1".into(),
                })
            })
            .await;
        // The new process knows no digests, so it requests nothing until the Node asks.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(world.cloud.issued_grants().len(), 1);
        world.node.deliveries.need(&execution);
        world
            .timeline
            .until(|events| received(events, &execution) == 2)
            .await;
        world.node.deliveries.complete(&execution);
        world
            .timeline
            .until(|events| {
                events.contains(&Event::DeliveryAck {
                    execution: execution.as_str().into(),
                    sequence: 1,
                })
            })
            .await;
        assert!(matches!(
            record(&world, "run")
                .result
                .and_then(|result| result.outcome),
            Some(proto::execution_result::Outcome::RevisionDelivered(_))
        ));
        assert_eq!(
            world
                .timeline
                .events()
                .iter()
                .filter(|e| matches!(e, Event::DeliveryStarted { .. }))
                .count(),
            1,
            "a delivery the Node knows is never sent again"
        );
        assert_eq!(world.cloud.delivery_records().len(), 1);
    });
}

/// Cloud refusing grants ends the request without failing the delivery or the connection.
#[test]
fn refused_grants_never_fail_the_delivery() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = delivering(&world, "run").await;
        let before = world.timeline.events().len();
        world.cloud.refuse_grants(true);
        world.node.deliveries.need(&execution);
        world
            .timeline
            .until(|events| {
                events.contains(&Event::GrantRefused {
                    execution: execution.as_str().into(),
                })
            })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(world.node.deliveries.grants().is_empty());
        assert_eq!(record(&world, "run").result, None);
        assert!(
            !world.timeline.events()[before..].contains(&Event::SessionEnded),
            "a refusal must not end the connection"
        );
        world.cloud.refuse_grants(false);
        world.node.deliveries.need(&execution);
        world
            .timeline
            .until(|events| received(events, &execution) == 1)
            .await;
    });
}

/// An unfinished delivery keeps the sandbox busy without breaking clone polling; once settled the
/// sandbox can be stopped.
#[test]
fn quiesce_counts_unfinished_deliveries() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = delivering(&world, "run").await;
        let before = world.timeline.events().len();
        let stop = world.cloud.queue(proto::OperationKind::Stop);
        settled(&world, &stop, proto::OperationState::Failed).await;
        let events = world.timeline.events()[before..].to_vec();
        assert!(events.contains(&Event::Idle { idle: false }));
        assert!(
            !events.contains(&Event::SessionEnded),
            "pending lists must not conflict over the delivery record: {events:#?}"
        );
        world.node.deliveries.complete(&execution);
        world
            .timeline
            .until(|events| {
                events.contains(&Event::DeliveryAck {
                    execution: execution.as_str().into(),
                    sequence: 1,
                })
            })
            .await;
        let stop = world.cloud.queue(proto::OperationKind::Stop);
        settled(&world, &stop, proto::OperationState::Succeeded).await;
        assert!(
            world
                .timeline
                .events()
                .contains(&Event::Idle { idle: true })
        );
    });
}

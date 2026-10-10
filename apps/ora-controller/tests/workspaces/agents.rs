//! Session production wiring through the real Cloud adapter and WebSocket transport.
use super::*;
use pretty_assertions::assert_eq;
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
struct AgentData {
    starts: HashMap<ExecutionId, StartAgentSessionMessage>,
    outbox: BTreeMap<(String, u64), NodeToControllerMessage>,
    terminal: HashMap<ExecutionId, AgentSessionResult>,
    counts: HashMap<ExecutionId, u64>,
    commands: HashSet<CommandId>,
}
#[derive(Clone, Default)]
pub(super) struct AgentNode {
    data: Arc<Mutex<AgentData>>,
    pub(super) enabled: Arc<AtomicBool>,
    pub(super) model_proxy: Arc<AtomicBool>,
    pub(super) drop_reply: Arc<AtomicBool>,
    suppress_replies: Arc<AtomicBool>,
    forged_reply: Arc<AtomicBool>,
}
impl AgentNode {
    /// Preserves the start and verifies the controlled envelope before accepting the execution.
    pub(super) fn start(&self, value: ControlledStartAgentSession, timeline: &Timeline) {
        value.validate().unwrap();
        let mut data = self.data.lock().unwrap();
        let command = value.command;
        if let Some(old) = data.starts.get(&command.execution_id) {
            assert_eq!(old, &command);
        }
        data.starts
            .insert(command.execution_id.clone(), command.clone());
        drop(data);
        timeline.push(Event::AgentStarted {
            execution: command.execution_id.as_str().into(),
        });
    }
    /// The start command the Node accepted for `execution`.
    pub(super) fn started(&self, execution: &ExecutionId) -> StartAgentSessionMessage {
        self.data.lock().unwrap().starts[execution].clone()
    }
    /// Supplies a status without manufacturing a terminal event receipt.
    pub(super) fn status(&self, execution: &ExecutionId) -> Option<ExecutionState> {
        let data = self.data.lock().unwrap();
        data.starts.get(execution).map(|_| {
            data.terminal
                .get(execution)
                .map_or(ExecutionState::Running, |r| {
                    ExecutionState::Completed(ExecutionResult::AgentSession(r.clone()))
                })
        })
    }
    /// Replays exactly the unacknowledged envelope once per connection.
    pub(super) fn next(
        &self,
        sent: &mut HashSet<(String, u64)>,
    ) -> Option<NodeToControllerMessage> {
        let data = self.data.lock().unwrap();
        let (key, event) = data.outbox.iter().find(|(key, _)| !sent.contains(*key))?;
        sent.insert(key.clone());
        Some(event.clone())
    }
    /// Records only acknowledgements corresponding to an actual retained event.
    pub(super) fn ack(&self, ack: &EventAckMessage, timeline: &Timeline) {
        let mut data = self.data.lock().unwrap();
        if data
            .outbox
            .remove(&(ack.execution_id.as_str().into(), ack.sequence.value()))
            .is_some()
        {
            drop(data);
            timeline.push(Event::AgentAck {
                execution: ack.execution_id.as_str().into(),
                sequence: ack.sequence.value(),
            });
        }
    }
    /// Appends a settled record to the Node-owned outbox.
    fn emit(&self, execution: &ExecutionId, size: usize) {
        let mut data = self.data.lock().unwrap();
        let start = data.starts.get(execution).unwrap().clone();
        let sequence = data.counts.entry(execution.clone()).or_default();
        *sequence += 1;
        let sequence = *sequence;
        data.outbox.insert(
            (execution.as_str().into(), sequence),
            NodeToControllerMessage::ThreadEvent(ThreadEventMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id,
                execution_id: execution.clone(),
                sequence: Sequence::new(sequence),
                payload: ThreadEvent {
                    turn_id: Some(TurnId::new("first-turn")),
                    record: serde_json::from_value(json!({"text": "x".repeat(size)})).unwrap(),
                    truncated: false,
                },
            }),
        );
    }
    /// Seals the Node session; the next status may expose this before its event is taken over.
    fn end(&self, execution: &ExecutionId) {
        let mut data = self.data.lock().unwrap();
        let start = data.starts.get(execution).unwrap().clone();
        let result = AgentSessionResult::AgentSessionEnded(AgentSessionEnded {
            node: NodeRuntimeIdentity {
                node_id: node_id(),
                incarnation_id: NodeIncarnationId::new("incarnation-1"),
            },
            reason: AgentSessionEndReason::UserEnded,
            detail: None,
        });
        data.terminal.insert(execution.clone(), result.clone());
        let sequence = data.counts.entry(execution.clone()).or_default();
        *sequence += 1;
        let sequence = *sequence;
        data.outbox.insert(
            (execution.as_str().into(), sequence),
            NodeToControllerMessage::AgentSessionEnded(AgentSessionEndedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id,
                execution_id: execution.clone(),
                sequence: Sequence::new(sequence),
                payload: result,
            }),
        );
    }
    /// Deduplicates effects while permitting a test to lose the original acceptance reply.
    pub(super) fn command(
        &self,
        command: AgentCommand,
        timeline: &Timeline,
    ) -> Option<NodeToControllerMessage> {
        let mut data = self.data.lock().unwrap();
        assert!(data.starts.contains_key(command.execution()));
        data.commands.insert(command.id().clone());
        let ended = data.terminal.contains_key(command.execution());
        drop(data);
        timeline.push(Event::CommandReceived {
            id: command.id().as_str().into(),
        });
        if self.drop_reply.swap(false, Ordering::SeqCst)
            || self.suppress_replies.load(Ordering::SeqCst)
        {
            return None;
        }
        Some(if ended {
            NodeToControllerMessage::SessionCommandRejected(SessionCommandRejectedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: command.operation().clone(),
                execution_id: command.execution().clone(),
                payload: SessionCommandRejected {
                    command_id: command.id().clone(),
                    reason: SessionCommandRejection::SessionEnded,
                },
            })
        } else {
            NodeToControllerMessage::SessionCommandAccepted(SessionCommandAcceptedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: if self.forged_reply.swap(false, Ordering::SeqCst) {
                    OperationId::new("foreign-operation")
                } else {
                    command.operation().clone()
                },
                execution_id: command.execution().clone(),
                payload: SessionCommandAccepted {
                    command_id: command.id().clone(),
                },
            })
        })
    }
}

/// Builds a real sandbox connection, then registers an Agent work item through ClaimWork.
async fn started(world: &World, run: &str) -> ExecutionId {
    world.cloud.queue_agent(run);
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
    ExecutionId::new(
        world
            .cloud
            .agent_records()
            .iter()
            .find(|r| r.operation_id == run)
            .unwrap()
            .execution_id
            .clone(),
    )
}
/// Starts a Workspace before testing its later IssueRun execution.
async fn workspace(world: &World) {
    world.node.agents.enabled.store(true, Ordering::SeqCst);
    let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
    settled(world, &create, proto::OperationState::Succeeded).await;
}

/// A Node can serve Echo while model-bound work waits for the additional negotiated capability.
#[test]
fn model_bound_session_requires_model_proxy_capability() {
    scenario("main", |world| async move {
        workspace(&world).await;
        world.cloud.queue_bound_agent("model-run", "binding-1");
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(world.cloud.agent_records(), Vec::new());
        assert_eq!(world.node.agents.data.lock().unwrap().starts.len(), 0);
    });
}

/// The production Cloud adapter carries the frozen reference unchanged into the controlled start.
#[test]
fn model_binding_is_preserved_through_controller_dispatch() {
    scenario("main", |world| async move {
        world.node.agents.model_proxy.store(true, Ordering::SeqCst);
        workspace(&world).await;
        world.cloud.queue_bound_agent("model-run", "binding-1");
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|event| matches!(event, Event::AgentStarted { .. }))
            })
            .await;
        let starts = world.node.agents.data.lock().unwrap();
        assert_eq!(
            starts
                .starts
                .values()
                .next()
                .unwrap()
                .payload
                .spec
                .model_binding_id,
            Some(ModelBindingId::new("binding-1"))
        );
    });
}
/// Waits for an exact ACK, never treating a Cloud query or log as receipt.
async fn acked(world: &World, execution: &ExecutionId, sequence: u64) {
    world
        .timeline
        .until(|events| {
            events.contains(&Event::AgentAck {
                execution: execution.as_str().into(),
                sequence,
            })
        })
        .await;
}

/// A delayed Thread write cannot acknowledge or settle a later terminal, even after a query.
#[test]
fn session_events_commit_before_ack_and_terminal_waits_for_thread() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = started(&world, "run").await;
        let before = world.timeline.events().len();
        world.cloud.block_agent("run", true);
        world.node.agents.emit(&execution, 12);
        world.node.agents.emit(&execution, 14);
        world.node.agents.end(&execution);
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::AgentBatchAttempt { run, .. } if run == "run"))
            })
            .await;
        // A blocked RPC outlives the transport deadline while heartbeats and status still flow.
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(world.cloud.agent_records()[0].result.is_none());
        assert!(
            !world.timeline.events()[before..]
                .iter()
                .any(|e| matches!(e, Event::AgentAck { .. } | Event::SessionEnded)),
            "unexpected early receipt or disconnect: {:?}",
            world.timeline.events()
        );
        world.cloud.block_agent("run", false);
        acked(&world, &execution, 3).await;
        let events = world.timeline.events();
        assert!(
            position(
                &events,
                &Event::ThreadTaken {
                    run: "run".into(),
                    through: 2
                }
            ) < position(
                &events,
                &Event::AgentAck {
                    execution: execution.as_str().into(),
                    sequence: 1
                }
            )
        );
        assert!(
            position(
                &events,
                &Event::ThreadTaken {
                    run: "run".into(),
                    through: 2
                }
            ) < position(&events, &Event::AgentEnded { run: "run".into() })
        );
        assert_eq!(world.cloud.thread_events(execution.as_str()).len(), 2);
    });
}

/// Independent executions continue taking over while one Cloud batch is held.
#[test]
fn slow_execution_does_not_block_another_thread() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let first = started(&world, "slow").await;
        let second = started(&world, "fast").await;
        world.cloud.block_agent("slow", true);
        world.node.agents.emit(&first, 10);
        world.node.agents.emit(&second, 10);
        acked(&world, &second, 1).await;
        assert!(world.cloud.thread_events(first.as_str()).is_empty());
        world.cloud.block_agent("slow", false);
        acked(&world, &first, 1).await;
    });
}

/// Outage, conflict and committed-but-lost replies all recover without duplicating records.
#[test]
fn batch_failures_and_lost_replies_preserve_exact_events() {
    for code in [
        tonic::Code::Unavailable,
        tonic::Code::Aborted,
        tonic::Code::DeadlineExceeded,
    ] {
        scenario("main", |world| async move {
            workspace(&world).await;
            let execution = started(&world, "run").await;
            if code == tonic::Code::DeadlineExceeded {
                world.cloud.lose_agent_reply();
            } else {
                world.cloud.fail_agent_batch(code);
            }
            world.node.agents.emit(&execution, 20);
            acked(&world, &execution, 1).await;
            assert_eq!(world.cloud.thread_events(execution.as_str()).len(), 1);
            let attempts: Vec<_> = world
                .timeline
                .events()
                .into_iter()
                .filter_map(|e| match e {
                    Event::AgentBatchAttempt { submission, .. } => Some(submission),
                    _ => None,
                })
                .collect();
            assert!(attempts.len() >= 2);
            if code == tonic::Code::DeadlineExceeded {
                assert_eq!(attempts[0], attempts[1]);
            }
            assert_eq!(world.node.agents.data.lock().unwrap().starts.len(), 1);
        });
    }
}

/// A fresh Controller replays Node evidence under the original execution identity.
#[test]
fn restart_replays_unacknowledged_events_without_starting_a_new_session() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = started(&world, "run").await;
        world.cloud.block_agent("run", true);
        world.node.agents.emit(&execution, 40);
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::AgentBatchAttempt { .. }))
            })
            .await;
        world.restart().await;
        world.cloud.block_agent("run", false);
        acked(&world, &execution, 1).await;
        assert_eq!(world.cloud.agent_records().len(), 1);
        assert_eq!(
            world
                .timeline
                .events()
                .iter()
                .filter(|e| matches!(e, Event::AgentStarted { .. }))
                .count(),
            1
        );
    });
}

/// A lost reply retries the same command and a terminal rejection still records delivery.
#[test]
fn commands_retry_in_order_and_closed_session_rejection_is_delivered() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = started(&world, "run").await;
        world.node.agents.drop_reply.store(true, Ordering::SeqCst);
        world.cloud.queue_agent_command("run", "one", false);
        world.cloud.queue_agent_command("run", "two", true);
        world
            .timeline
            .until(|_| world.cloud.delivered_commands().len() == 2)
            .await;
        let events = world.timeline.events();
        assert!(
            events
                .iter()
                .filter(|e| **e == Event::CommandReceived { id: "one".into() })
                .count()
                >= 2
        );
        assert!(
            position(&events, &Event::CommandDelivered { id: "one".into() })
                < position(&events, &Event::CommandReceived { id: "two".into() })
        );
        world.node.agents.end(&execution);
        acked(&world, &execution, 1).await;
        world.cloud.queue_agent_command("run", "late", false);
        world
            .timeline
            .until(|_| world.cloud.delivered_commands().contains("late"))
            .await;
        assert_eq!(world.node.agents.data.lock().unwrap().commands.len(), 3);
    });
}

/// Valid near-limit records must not exceed the gRPC server default batch size.
#[test]
fn large_records_are_split_into_grpc_sized_ordered_batches() {
    scenario("main", |world| async move {
        workspace(&world).await;
        let execution = started(&world, "run").await;
        for _ in 0..20 {
            world.node.agents.emit(&execution, 240_000);
        }
        acked(&world, &execution, 20).await;
        assert_eq!(world.cloud.thread_events(execution.as_str()).len(), 20);
        assert!(
            world
                .timeline
                .events()
                .iter()
                .filter(|e| matches!(e, Event::ThreadTaken { .. }))
                .count()
                >= 4
        );
    });
}

/// Rebuilding the Controller must resend an accepted-but-unconfirmed command before the next one.
#[test]
fn restart_redelivers_the_unconfirmed_command_before_later_turns() {
    scenario("main", |world| async move {
        workspace(&world).await;
        started(&world, "run").await;
        world
            .node
            .agents
            .suppress_replies
            .store(true, Ordering::SeqCst);
        world.cloud.queue_agent_command("run", "one", false);
        world.cloud.queue_agent_command("run", "two", false);
        world
            .timeline
            .until(|events| events.contains(&Event::CommandReceived { id: "one".into() }))
            .await;
        assert!(world.cloud.delivered_commands().is_empty());
        assert!(
            !world
                .timeline
                .events()
                .contains(&Event::CommandReceived { id: "two".into() })
        );
        world.restart().await;
        world
            .node
            .agents
            .suppress_replies
            .store(false, Ordering::SeqCst);
        world
            .timeline
            .until(|_| world.cloud.delivered_commands().len() == 2)
            .await;
        assert_eq!(world.node.agents.data.lock().unwrap().commands.len(), 2);
    });
}

/// Session responsibility prevents quiesce from destroying a Workspace that is still in use.
#[test]
fn quiesce_counts_unfinished_agent_sessions() {
    scenario("main", |world| async move {
        workspace(&world).await;
        started(&world, "run").await;
        let stop = world.cloud.queue(proto::OperationKind::Stop);
        settled(&world, &stop, proto::OperationState::Failed).await;
        let events = world.timeline.events();
        assert!(events.contains(&Event::Idle { idle: false }));
        assert!(!events.contains(&Event::Substrate {
            method: "PUT",
            kind: "sandbox_terminate"
        }));
    });
}

/// A known Node without Agent capability cannot consume queued session work.
#[test]
fn session_registration_waits_for_a_capable_handshake() {
    scenario("main", |world| async move {
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        world.cloud.queue_agent("run");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(world.cloud.agent_records().is_empty());
        world.node.agents.enabled.store(true, Ordering::SeqCst);
        world.restart().await;
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::AgentStarted { .. }))
            })
            .await;
        assert_eq!(world.cloud.agent_records().len(), 1);
    });
}

/// A historical registration cannot authorize a start while its current runtime input is closed.
#[test]
fn registered_sessions_wait_for_current_runtime_permission() {
    scenario("main", |world| async move {
        workspace(&world).await;
        world.cloud.close_agent_input(true);
        world.cloud.queue_agent("run");
        world
            .timeline
            .until(|events| events.contains(&Event::AgentRegistered { run: "run".into() }))
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !world
                .timeline
                .events()
                .iter()
                .any(|e| matches!(e, Event::AgentStarted { .. }))
        );
        world.cloud.close_agent_input(false);
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::AgentStarted { .. }))
            })
            .await;
        assert_eq!(world.cloud.agent_records().len(), 1);
    });
}

/// A reply carrying a different operation never records delivery; reconnect resends the original.
#[test]
fn foreign_command_reply_is_not_delivery_evidence() {
    scenario("main", |world| async move {
        workspace(&world).await;
        started(&world, "run").await;
        let before = world.timeline.events().len();
        world.node.agents.forged_reply.store(true, Ordering::SeqCst);
        world.cloud.queue_agent_command("run", "one", false);
        world
            .timeline
            .until(|_| world.cloud.delivered_commands().contains("one"))
            .await;
        let events = world.timeline.events()[before..].to_vec();
        assert!(
            position(&events, &Event::SessionEnded)
                < position(&events, &Event::CommandDelivered { id: "one".into() })
        );
        assert!(
            events
                .iter()
                .filter(|e| **e == Event::CommandReceived { id: "one".into() })
                .count()
                >= 2
        );
        assert_eq!(world.node.agents.data.lock().unwrap().commands.len(), 1);
    });
}

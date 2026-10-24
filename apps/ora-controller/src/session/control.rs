//! Authority calls run outside the frame pump so a slow Cloud cannot starve heartbeats or Thread queues.
use super::*;
use tokio::{sync::mpsc, task::JoinSet};

/// What a session tells its owner beyond its own result. A static Node's handshake proves its
/// configured identity to the store; the Cloud adapter's sandbox sessions report the handshake to
/// Cloud and stop a Workspace's clone step when the Node cannot tell an execution's outcome. Callbacks run on the session task and must not
/// block it.
pub(crate) trait SessionObserver: Send + Sync {
    /// The handshake completed with this Node incarnation; the session is live from here on.
    fn established(&self, node: &NodeRuntimeIdentity);
    /// Records the negotiated capabilities before publishing the connected identity.
    fn capabilities(
        &self,
        _capabilities: &[NodeCapability],
        _model_adapter: Option<&ModelAdapterCapability>,
    ) {
    }
    /// The Node still answers `Unknown` for an execution after this connection retransmitted its
    /// original command once. It may be a retained uncertain attempt or a command the Node has not
    /// admitted yet; the owner decides how long that may last.
    fn unresolved(&self, execution: &ExecutionId);
    /// Cloud refused this validated delivery terminal without issuing a receipt.
    fn terminal_refused(&self, execution: &ExecutionId);
    /// A real terminal takeover receipt, rather than a status hint, resolves a refusal.
    fn terminal_accepted(&self, execution: &ExecutionId);
    /// Refused terminals from an earlier transport connection still await their exact receipts.
    fn refused_executions(&self) -> Vec<ExecutionId>;
    /// The Node reported the execution as accepted, running or completed, which ends any earlier
    /// unresolved report for it.
    fn answered(&self, execution: &ExecutionId);
}

/// The observer of sessions nobody watches.
pub(crate) struct Unobserved;

impl SessionObserver for Unobserved {
    fn established(&self, _node: &NodeRuntimeIdentity) {}

    fn unresolved(&self, _execution: &ExecutionId) {}

    fn answered(&self, _execution: &ExecutionId) {}

    fn terminal_refused(&self, _execution: &ExecutionId) {}

    fn terminal_accepted(&self, _execution: &ExecutionId) {}

    fn refused_executions(&self) -> Vec<ExecutionId> {
        Vec::new()
    }
}

/// Effects returned to the connection owner after durable coordination.
pub(super) enum Action {
    Send(Box<ControllerToNodeMessage>),
    Answered(ExecutionId),
    TerminalRefused(ExecutionId),
    TerminalAccepted(ExecutionId),
    Unresolved(ExecutionId),
}

/// Connection-local reconciliation state; a refused terminal outranks every status hint.
enum ExecutionControl {
    Retransmitted,
    /// A prior connection was refused; statuses cannot authorize redispatch before its replay.
    AwaitingRefusedTerminal,
    TerminalRefused {
        event: Box<RevisionResultMessage>,
        /// Retain Cloud's original verdict with the unresolved execution for diagnostics.
        detail: String,
    },
}

/// Two bounded workers for polling and reconciliation; both are cancelled with their connection.
pub(super) struct Control {
    pub(super) input: mpsc::Sender<NodeToControllerMessage>,
    pub(super) outgoing: mpsc::Receiver<Action>,
    pub(super) tasks: JoinSet<Result<(), Error>>,
}
impl Control {
    /// Starts polling and reply processing independently of the transport's I/O deadline.
    pub(super) fn new<S: CoordinationStore>(
        store: S,
        identity: NodeRuntimeIdentity,
        period: Duration,
        capabilities: Vec<NodeCapability>,
        model_adapter: Option<ModelAdapterCapability>,
        workers: Workers,
    ) -> Self {
        let (input, receive) = mpsc::channel(256);
        let (output, outgoing) = mpsc::channel(256);
        let mut tasks = JoinSet::new();
        tasks.spawn(poll(
            store.clone(),
            identity.node_id.clone(),
            period,
            Polled {
                agents: capabilities.contains(&NodeCapability::AgentSession),
                deliveries: capabilities.contains(&NodeCapability::RevisionDelivery),
            },
            output.clone(),
        ));
        tasks.spawn(reconcile(
            store,
            identity,
            capabilities,
            model_adapter,
            receive,
            workers,
            output,
        ));
        Self {
            input,
            outgoing,
            tasks,
        }
    }
}

/// Names the poll step that failed. Every one of these errors ends the whole session, and the
/// conflict class they usually carry says nothing about which call produced it.
fn step_failed(step: &'static str, error: Error) -> Error {
    ora_logging::ora_warn!(step, error = %error, "session poll step failed");
    error
}

/// Which optional execution families the Node negotiated and therefore may be polled.
struct Polled {
    agents: bool,
    deliveries: bool,
}

/// The connection's other workers that learn from validated status replies.
pub(super) struct Workers {
    pub(super) commands: mpsc::Sender<commands::Reply>,
    pub(super) grants: mpsc::Sender<grants::Request>,
    pub(super) refused: Vec<ExecutionId>,
}

/// Queries one execution per tick; bindings precede any execution that may need their authority.
async fn poll<S: CoordinationStore>(
    store: S,
    node: NodeId,
    period: Duration,
    polled: Polled,
    output: mpsc::Sender<Action>,
) -> Result<(), Error> {
    let mut tick = interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cursor = 0usize;
    loop {
        tick.tick().await;
        let bindings = store
            .runtime_bindings(&node)
            .await
            .map_err(|error| step_failed("runtime_bindings", error))?;
        for binding in bindings {
            if output
                .send(Action::Send(Box::new(
                    ControllerToNodeMessage::BindRuntime(binding),
                )))
                .await
                .is_err()
            {
                return Ok(());
            }
        }
        let mut commands: Vec<_> = store
            .pending_dispatches(&node)
            .await
            .map_err(|error| step_failed("pending_dispatches", error))?
            .into_iter()
            .map(|c| (c.operation_id, c.execution_id))
            .collect();
        commands.extend(
            store
                .pending_plugins(&node)
                .await
                .map_err(|error| step_failed("pending_plugins", error))?
                .into_iter()
                .map(|c| (c.operation_id().clone(), c.execution_id().clone())),
        );
        if polled.agents {
            commands.extend(
                store
                    .pending_agents(&node)
                    .await
                    .map_err(|error| step_failed("pending_agents", error))?
                    .into_iter()
                    .map(|c| (c.operation_id, c.execution_id)),
            );
        }
        if polled.deliveries {
            commands.extend(
                store
                    .pending_deliveries(&node)
                    .await
                    .map_err(|error| step_failed("pending_deliveries", error))?
                    .into_iter()
                    .map(|c| (c.operation_id, c.execution_id)),
            );
            commands.extend(
                store
                    .pending_scheduled_artifacts(&node)
                    .await
                    .map_err(|error| step_failed("pending_scheduled_artifacts", error))?
                    .into_iter()
                    .map(|c| (c.operation_id, c.execution_id)),
            );
        }
        if !commands.is_empty() {
            let (operation_id, execution_id) = commands[cursor % commands.len()].clone();
            cursor = cursor.wrapping_add(1);
            let query = ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id,
                execution_id,
                payload: GetExecutionStatus {
                    node_id: node.clone(),
                },
            });
            if output.send(Action::Send(Box::new(query))).await.is_err() {
                return Ok(());
            }
        }
    }
}

/// Reconciles ordinary results and status queries; Thread terminal events use their ordered relay.
async fn reconcile<S: CoordinationStore>(
    store: S,
    identity: NodeRuntimeIdentity,
    capabilities: Vec<NodeCapability>,
    model_adapter: Option<ModelAdapterCapability>,
    mut input: mpsc::Receiver<NodeToControllerMessage>,
    workers: Workers,
    output: mpsc::Sender<Action>,
) -> Result<(), Error> {
    let agent_capable = capabilities.contains(&NodeCapability::AgentSession);
    let delivery_capable = capabilities.contains(&NodeCapability::RevisionDelivery);
    let plugin_capable = capabilities.contains(&NodeCapability::PluginInstall);
    let mut executions: std::collections::HashMap<_, _> = workers
        .refused
        .iter()
        .cloned()
        .map(|id| (id, ExecutionControl::AwaitingRefusedTerminal))
        .collect();
    while let Some(message) = input.recv().await {
        let execution = match &message {
            NodeToControllerMessage::RevisionResult(event) => Some(&event.execution_id),
            NodeToControllerMessage::ExecutionStatus(status) => Some(&status.execution_id),
            _ => None,
        };
        if let Some(ExecutionControl::TerminalRefused { event, detail }) =
            execution.and_then(|id| executions.get(id))
        {
            // Suppress only replays of the original binding. Conflicting bodies, producers or
            // operations still fail the connection rather than disappearing behind isolation.
            let matches = match &message {
                NodeToControllerMessage::RevisionResult(duplicate) => duplicate == event.as_ref(),
                NodeToControllerMessage::ExecutionStatus(status) => {
                    status.operation_id == event.operation_id
                        && status.payload.node == identity
                        && match &status.payload.state {
                            ExecutionState::Completed(ExecutionResult::Revision(result)) => {
                                result.as_ref() == &event.payload
                            }
                            ExecutionState::Unknown
                            | ExecutionState::Accepted
                            | ExecutionState::Running => true,
                            ExecutionState::Completed(_) => false,
                        }
                }
                _ => false,
            };
            if !matches {
                ora_logging::ora_error!(
                    execution_id = %event.execution_id.as_str(),
                    operation_id = %event.operation_id.as_str(),
                    cloud_refusal = %detail,
                    "Conflicting terminal or status for refused delivery execution"
                );
                return Err(Error::Conflict);
            }
            continue;
        }
        let ack = match take_over(&store, &identity, &message).await {
            Ok(ack) => ack,
            Err(Error::TerminalTakeoverRefused(detail))
                if let NodeToControllerMessage::RevisionResult(event) = &message =>
            {
                executions.insert(
                    event.execution_id.clone(),
                    ExecutionControl::TerminalRefused {
                        event: Box::new(event.clone()),
                        detail,
                    },
                );
                output
                    .send(Action::TerminalRefused(event.execution_id.clone()))
                    .await
                    .map_err(|_| Error::Conflict)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        if let NodeToControllerMessage::ExecutionStatus(status) = &message
            && matches!(
                executions.get(&status.execution_id),
                Some(ExecutionControl::AwaitingRefusedTerminal)
            )
        {
            // The normal takeover boundary above still validates the status identity. Its
            // sequence-free answer cannot replace the retained terminal or permit a new start.
            continue;
        }
        if agent_capable
            && let NodeToControllerMessage::ExecutionStatus(status) = &message
            && status.payload.state != ExecutionState::Unknown
        {
            workers
                .commands
                .try_send(commands::Reply::Ready(status.execution_id.clone()))
                .map_err(|_| Error::Conflict)?;
        }
        // A running delivery found by this connection may have lost its grants with the previous
        // one; the grant worker decides whether a refresh is possible and dedups repeated polls.
        // The hint is advisory: while a slow grant call fills the queue the next poll repeats it,
        // and a closed queue means the worker already ended the connection.
        if delivery_capable
            && let NodeToControllerMessage::ExecutionStatus(status) = &message
            && matches!(
                status.payload.state,
                ExecutionState::Accepted | ExecutionState::Running
            )
        {
            let _ = workers.grants.try_send(grants::Request::Running {
                operation: status.operation_id.clone(),
                execution: status.execution_id.clone(),
            });
        }
        if let NodeToControllerMessage::RevisionResult(event) = &message {
            if matches!(
                executions.get(&event.execution_id),
                Some(ExecutionControl::AwaitingRefusedTerminal)
            ) {
                executions.remove(&event.execution_id);
            }
            output
                .send(Action::TerminalAccepted(event.execution_id.clone()))
                .await
                .map_err(|_| Error::Conflict)?;
        }
        if let NodeToControllerMessage::ScheduledArtifactsResult(event) = &message {
            output
                .send(Action::TerminalAccepted(event.execution_id.clone()))
                .await
                .map_err(|_| Error::Conflict)?;
        }
        if let NodeToControllerMessage::PluginsResult(event) = &message {
            output
                .send(Action::Answered(event.execution_id.clone()))
                .await
                .map_err(|_| Error::Conflict)?;
        }
        if let NodeToControllerMessage::CloneResult(event) = &message {
            // Cloud has durably accepted the terminal fact; a prior Unknown is now resolved.
            output
                .send(Action::Answered(event.execution_id.clone()))
                .await
                .map_err(|_| Error::Conflict)?;
        }
        if let NodeToControllerMessage::ExecutionStatus(status) = &message
            && status.payload.state != ExecutionState::Unknown
        {
            output
                .send(Action::Answered(status.execution_id.clone()))
                .await
                .map_err(|_| Error::Conflict)?;
        }
        let reply = if let Some(ack) = ack {
            Some(ControllerToNodeMessage::EventAck(ack))
        } else if let NodeToControllerMessage::ExecutionStatus(status) = &message
            && status.payload.state == ExecutionState::Unknown
        {
            if !executions.contains_key(&status.execution_id) {
                let command = if let Some(agent) = store
                    .original_agent_dispatch(&identity, &status.operation_id, &status.execution_id)
                    .await?
                {
                    // A configured session retransmits only to an incarnation that still admits it.
                    if agent_capable
                        && (agent.payload.spec.model_binding_id.is_none()
                            || capabilities.contains(&NodeCapability::ModelProxy))
                        && agent.payload.spec.agent_configuration.as_ref().is_none_or(
                            |configuration| {
                                model_adapter
                                    .as_ref()
                                    .is_some_and(|adapter| adapter.admits(configuration))
                            },
                        )
                    {
                        store.dispatch_agent(agent).await?
                    } else {
                        None
                    }
                } else if let Some(delivery) = store
                    .original_delivery_dispatch(
                        &identity,
                        &status.operation_id,
                        &status.execution_id,
                    )
                    .await?
                {
                    if delivery_capable {
                        store.dispatch_delivery(delivery).await?
                    } else {
                        None
                    }
                } else if let Some(upload) = store
                    .original_scheduled_artifacts_dispatch(
                        &identity,
                        &status.operation_id,
                        &status.execution_id,
                    )
                    .await?
                {
                    if delivery_capable {
                        store.dispatch_scheduled_artifacts(upload).await?
                    } else {
                        None
                    }
                } else if let Some(plugin) = store
                    .original_plugin_dispatch(&identity, &status.operation_id, &status.execution_id)
                    .await?
                {
                    if plugin_capable {
                        store.dispatch_plugins(plugin).await?
                    } else {
                        None
                    }
                } else if store.result(&status.execution_id).await?.is_none() {
                    store
                        .dispatch_message(
                            store
                                .original_dispatch(
                                    &identity,
                                    &status.operation_id,
                                    &status.execution_id,
                                )
                                .await?,
                        )
                        .await?
                } else {
                    None
                };
                if command.is_some() {
                    executions.insert(status.execution_id.clone(), ExecutionControl::Retransmitted);
                }
                command
            } else {
                output
                    .send(Action::Unresolved(status.execution_id.clone()))
                    .await
                    .map_err(|_| Error::Conflict)?;
                None
            }
        } else {
            None
        };
        if let Some(reply) = reply {
            output
                .send(Action::Send(Box::new(reply)))
                .await
                .map_err(|_| Error::Conflict)?;
        }
    }
    Ok(())
}

//! Authority calls run outside the frame pump so a slow Cloud cannot starve heartbeats or Thread queues.
use super::*;
use tokio::{sync::mpsc, task::JoinSet};

/// Effects returned to the connection owner after durable coordination.
pub(super) enum Action {
    Send(Box<ControllerToNodeMessage>),
    Answered(ExecutionId),
    Unresolved(ExecutionId),
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
        commands: mpsc::Sender<commands::Reply>,
    ) -> Self {
        let (input, receive) = mpsc::channel(256);
        let (output, outgoing) = mpsc::channel(256);
        let mut tasks = JoinSet::new();
        tasks.spawn(poll(
            store.clone(),
            identity.node_id.clone(),
            period,
            capabilities.contains(&NodeCapability::AgentSession),
            output.clone(),
        ));
        tasks.spawn(reconcile(
            store,
            identity,
            capabilities,
            receive,
            commands,
            output,
        ));
        Self {
            input,
            outgoing,
            tasks,
        }
    }
}

/// Queries one execution per tick; bindings precede any execution that may need their authority.
async fn poll<S: CoordinationStore>(
    store: S,
    node: NodeId,
    period: Duration,
    agent_capable: bool,
    output: mpsc::Sender<Action>,
) -> Result<(), Error> {
    let mut tick = interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cursor = 0usize;
    loop {
        tick.tick().await;
        for binding in store.runtime_bindings(&node).await? {
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
            .await?
            .into_iter()
            .map(|c| (c.operation_id, c.execution_id))
            .collect();
        commands.extend(
            store
                .pending_plugins(&node)
                .await?
                .into_iter()
                .map(|c| (c.operation_id().clone(), c.execution_id().clone())),
        );
        if agent_capable {
            commands.extend(
                store
                    .pending_agents(&node)
                    .await?
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
    mut input: mpsc::Receiver<NodeToControllerMessage>,
    commands: mpsc::Sender<commands::Reply>,
    output: mpsc::Sender<Action>,
) -> Result<(), Error> {
    let agent_capable = capabilities.contains(&NodeCapability::AgentSession);
    let plugin_capable = capabilities.contains(&NodeCapability::PluginInstall);
    let mut retransmitted = std::collections::HashSet::new();
    while let Some(message) = input.recv().await {
        let ack = take_over(&store, &identity, &message).await?;
        if agent_capable
            && let NodeToControllerMessage::ExecutionStatus(status) = &message
            && status.payload.state != ExecutionState::Unknown
        {
            commands
                .try_send(commands::Reply::Ready(status.execution_id.clone()))
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
            if !retransmitted.contains(&status.execution_id) {
                let command = if let Some(agent) = store
                    .original_agent_dispatch(&identity, &status.operation_id, &status.execution_id)
                    .await?
                {
                    if agent_capable {
                        store.dispatch_agent(agent).await?
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
                    retransmitted.insert(status.execution_id.clone());
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

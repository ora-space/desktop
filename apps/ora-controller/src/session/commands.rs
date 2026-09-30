//! Ordered command delivery independent of Thread takeover and heartbeat receipt.
use super::*;
use std::collections::{HashMap, HashSet};
use tokio::{sync::mpsc, task::JoinSet};

/// The session validates execution status before exposing it to the delivery worker.
pub(super) enum Reply {
    Ready(ExecutionId),
    Command {
        operation: OperationId,
        execution: ExecutionId,
        command: CommandId,
    },
}

/// Connection-local correlation only; Cloud retains every command until a Node reply is recorded.
pub(super) struct Commands {
    pub(super) input: mpsc::Sender<Reply>,
    pub(super) outgoing: mpsc::Receiver<ControllerToNodeMessage>,
    pub(super) tasks: JoinSet<Result<(), Error>>,
}
impl Commands {
    /// Starts one bounded delivery worker when the Node advertises Agent support.
    pub(super) fn new<S: CoordinationStore>(
        store: S,
        node: NodeRuntimeIdentity,
        period: Duration,
        enabled: bool,
    ) -> Self {
        let (send, receive) = mpsc::channel(256);
        let (output, outgoing) = mpsc::channel(128);
        let mut tasks = JoinSet::new();
        if enabled {
            tasks.spawn(run(store, node, period, receive, output));
        }
        Self {
            input: send,
            outgoing,
            tasks,
        }
    }
}

/// Sends only a run's head until Cloud records its reply. Repeated sends retain the same ID.
async fn run<S: CoordinationStore>(
    store: S,
    node: NodeRuntimeIdentity,
    period: Duration,
    mut input: mpsc::Receiver<Reply>,
    output: mpsc::Sender<ControllerToNodeMessage>,
) -> Result<(), Error> {
    let mut tick = interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ready = HashSet::new();
    let mut sent: HashMap<CommandId, AgentCommand> = HashMap::new();
    let mut delivered: HashMap<CommandId, (OperationId, ExecutionId)> = HashMap::new();
    loop {
        tokio::select! {
            biased;
            reply = input.recv() => match reply {
                None => return Ok(()),
                Some(Reply::Ready(execution)) => { ready.insert(execution); }
                Some(Reply::Command { operation, execution, command }) => {
                    if let Some(identity) = delivered.get(&command) {
                        if identity != &(operation, execution) { return Err(Error::Conflict); }
                        continue;
                    }
                    let Some(original) = sent.get(&command) else { return Err(Error::Conflict); };
                    if original.operation() != &operation || original.execution() != &execution { return Err(Error::Conflict); }
                    loop {
                        match store.agent_command_delivered(original).await {
                            Ok(()) => break,
                            Err(Error::Unavailable(_)) => tokio::time::sleep(Duration::from_millis(250)).await,
                            Err(error) => return Err(error),
                        }
                    }
                    // Duplicate replies need only compact identities, not retained user content.
                    sent.remove(&command);
                    delivered.insert(command, (operation, execution));
                    tick.reset_immediately();
                }
            },
            _ = store.wait_agent_command_hint() => tick.reset_immediately(),
            _ = tick.tick() => {
                let commands = store.pending_agent_commands(&node.node_id).await?;
                let mut runs = HashSet::new();
                for command in commands {
                    if !runs.insert(command.execution().clone()) { continue; }
                    if !ready.contains(command.execution()) {
                        let query = ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION, operation_id: command.operation().clone(), execution_id: command.execution().clone(), payload: GetExecutionStatus { node_id: node.node_id.clone() },
                        });
                        if output.send(query).await.is_err() { return Ok(()); }
                        continue;
                    }
                    if sent.get(command.id()).is_some_and(|old| old != &command) { return Err(Error::Conflict); }
                    let frame = command.message();
                    sent.insert(command.id().clone(), command);
                    if output.send(frame).await.is_err() { return Ok(()); }
                }
            }
        }
    }
}

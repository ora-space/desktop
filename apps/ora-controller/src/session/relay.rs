//! Per-execution ordered takeover workers. Dropping the connection aborts every worker; Node's
//! unacknowledged ledger is the recovery source, so no in-memory queue is authoritative.
use super::*;
use std::collections::HashMap;
use tokio::{sync::mpsc, task::JoinSet};

/// An actual event, never a query result that could skip undispatched Thread records.
#[derive(Clone)]
pub(super) enum Event {
    Thread(ThreadEventMessage),
    End(AgentSessionEndedMessage),
}
impl Event {
    /// Identifies the independent queue receiving this event.
    fn execution(&self) -> &ExecutionId {
        match self {
            Self::Thread(v) => &v.execution_id,
            Self::End(v) => &v.execution_id,
        }
    }
    /// Builds an exact acknowledgement, used only after the associated write succeeds.
    fn ack(&self, node: &NodeId) -> Receipt {
        let (operation, execution, sequence) = match self {
            Self::Thread(v) => (&v.operation_id, &v.execution_id, v.sequence),
            Self::End(v) => (&v.operation_id, &v.execution_id, v.sequence),
        };
        let ack = EventAckMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: operation.clone(),
            execution_id: execution.clone(),
            sequence,
            payload: EventAck {
                node_id: node.clone(),
            },
        };
        match self {
            Self::Thread(_) => Receipt::Thread(ack),
            Self::End(_) => Receipt::Terminal(ack),
        }
    }
}

/// The terminal receipt also retires connection-local uncertainty from older status replies.
pub(super) enum Receipt {
    Thread(EventAckMessage),
    Terminal(EventAckMessage),
}

/// Bounds each queue by Node's in-flight window; workers and replies belong to one connection.
pub(super) struct Relay {
    queues: HashMap<ExecutionId, mpsc::Sender<Event>>,
    pub(super) tasks: JoinSet<Result<(), Error>>,
    replies: mpsc::Sender<Receipt>,
    pub(super) outgoing: mpsc::Receiver<Receipt>,
}
impl Relay {
    /// Creates no tasks until a real event is received.
    pub(super) fn new() -> Self {
        let (replies, outgoing) = mpsc::channel(256);
        Self {
            queues: HashMap::new(),
            tasks: JoinSet::new(),
            replies,
            outgoing,
        }
    }
    /// Enqueues without blocking other executions. Overflow violates the negotiated event window.
    pub(super) fn push<S: CoordinationStore>(
        &mut self,
        store: &S,
        node: &NodeRuntimeIdentity,
        event: Event,
    ) -> Result<(), SessionError> {
        let queue = self
            .queues
            .entry(event.execution().clone())
            .or_insert_with(|| {
                let (send, receive) = mpsc::channel(256);
                self.tasks.spawn(run(
                    store.clone(),
                    node.clone(),
                    receive,
                    self.replies.clone(),
                ));
                send
            });
        queue.try_send(event).map_err(|_| {
            SessionError::Protocol("session event queue exceeded its window or ended".into())
        })
    }
}

/// Batches at most 64 records or 100 ms, additionally staying below gRPC's default message limit.
async fn run<S: CoordinationStore>(
    store: S,
    node: NodeRuntimeIdentity,
    mut input: mpsc::Receiver<Event>,
    replies: mpsc::Sender<Receipt>,
) -> Result<(), Error> {
    let mut pending = None;
    loop {
        let first = match pending.take() {
            Some(event) => event,
            None => match input.recv().await {
                Some(event) => event,
                None => return Ok(()),
            },
        };
        let mut events = vec![first];
        if matches!(&events[0], Event::Thread(_)) {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
            let mut bytes = 0;
            while events.len() < 64 {
                if let Some(Event::Thread(event)) = events.last() {
                    bytes += serde_json::to_vec(&event.payload)?.len();
                }
                if bytes >= 1024 * 1024 {
                    break;
                }
                match tokio::time::timeout_at(deadline, input.recv()).await {
                    Ok(Some(event @ Event::Thread(_))) => events.push(event),
                    Ok(Some(end @ Event::End(_))) => {
                        pending = Some(end);
                        break;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
        loop {
            let result = match &events[0] {
                Event::End(end) => store.take_over_agent_end(&node, end).await,
                Event::Thread(_) => {
                    let batch: Vec<_> = events
                        .iter()
                        .filter_map(|event| match event {
                            Event::Thread(v) => Some(v.clone()),
                            Event::End(_) => None,
                        })
                        .collect();
                    store.take_over_thread(&node, &batch).await
                }
            };
            match result {
                Ok(()) => break,
                // A definitely uncommitted batch remains in order; unknown writes already used
                // the adapter's identical submission retries and require transport replay.
                Err(Error::Unavailable(_)) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(error) => return Err(error),
            }
        }
        for event in events {
            if replies.send(event.ack(&node.node_id)).await.is_err() {
                return Ok(());
            }
        }
    }
}

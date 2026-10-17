//! One connection owns its in-flight window; reconnect discards cursors and replays durable events.
use ora_node_db::EventCursor;
use ora_node_protocol::{EventAckMessage, ExecutionId, NodeToControllerMessage, Sequence};
use std::collections::{BTreeSet, HashMap};

const WINDOW: usize = 256;

#[derive(Default)]
pub(super) struct Delivery {
    executions: HashMap<ExecutionId, InFlight>,
}

#[derive(Default)]
struct InFlight {
    after: u64,
    pending: BTreeSet<u64>,
}

impl Delivery {
    /// A snapshot skips already reserved frames and executions with no free slot.
    pub(super) fn cursors(&self) -> Vec<EventCursor> {
        self.executions
            .iter()
            .map(|(execution, state)| EventCursor {
                execution: execution.clone(),
                after: state.after,
                remaining: WINDOW - state.pending.len(),
            })
            .collect()
    }

    /// Reservations include queued writes, keeping slow peers within the same hard window.
    pub(super) fn reserve(&mut self, message: &NodeToControllerMessage) -> bool {
        let Some((execution, sequence)) = event_key(message) else {
            return true;
        };
        let state = self.executions.entry(execution.clone()).or_default();
        if state.pending.len() >= WINDOW || sequence.value() <= state.after {
            return false;
        }
        state.after = sequence.value();
        state.pending.insert(sequence.value());
        true
    }

    /// Only the worker's successful durable ACK releases a slot; unknown ACKs cannot grow it.
    pub(super) fn acknowledged(&mut self, ack: &EventAckMessage) {
        if let Some(state) = self.executions.get_mut(&ack.execution_id) {
            state.pending.remove(&ack.sequence.value());
        }
    }
}

/// Every event family shares exact ACK semantics; replies and heartbeats have no delivery slot.
fn event_key(message: &NodeToControllerMessage) -> Option<(&ExecutionId, Sequence)> {
    match message {
        NodeToControllerMessage::ThreadEvent(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::AgentSessionEnded(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::CloneResult(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::PluginsResult(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::WorktreeReady(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::WorktreeFailed(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::WorktreeRemoved(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::WorktreeRemovalFailed(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::RevisionResult(m) => Some((&m.execution_id, m.sequence)),
        NodeToControllerMessage::HelloAccepted(_)
        | NodeToControllerMessage::Heartbeat(_)
        | NodeToControllerMessage::ExecutionStatus(_)
        | NodeToControllerMessage::RuntimeControlState(_)
        | NodeToControllerMessage::SessionCommandAccepted(_)
        | NodeToControllerMessage::SessionCommandRejected(_)
        | NodeToControllerMessage::UploadGrantNeeded(_) => None,
    }
}

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};

use crate::PluginRuntimeError;
use crate::protocol::{PluginNotification, PluginRegistration};

pub(crate) type PendingResult = Result<Value, PluginRuntimeError>;

// Bound stale request ids per plugin generation so a misbehaving plugin cannot grow host memory.
const ABANDONED_REQUEST_CAPACITY: usize = 256;

/// Classifies a response id without conflating a timed-out request with a foreign id.
pub(crate) enum ResponseRequest {
    Pending(oneshot::Sender<PendingResult>),
    Abandoned,
    Unmatched,
}

/// Keeps live control calls separate from bounded tombstones for calls that timed out.
#[derive(Default)]
pub(crate) struct PendingRequests {
    active: HashMap<u64, oneshot::Sender<PendingResult>>,
    abandoned: VecDeque<u64>,
}

impl PendingRequests {
    /// Registers one control call until it receives a response or times out.
    pub(crate) fn insert(&mut self, request_id: u64, sender: oneshot::Sender<PendingResult>) {
        self.active.insert(request_id, sender);
    }

    /// Removes a call that was never written and therefore cannot produce a valid response.
    pub(crate) fn remove_active(&mut self, request_id: u64) {
        self.active.remove(&request_id);
    }

    /// Retires a timed-out call while retaining enough identity to ignore its late response.
    pub(crate) fn abandon(&mut self, request_id: u64) {
        if self.active.remove(&request_id).is_none() {
            return;
        }
        if self.abandoned.len() == ABANDONED_REQUEST_CAPACITY {
            self.abandoned.pop_front();
        }
        self.abandoned.push_back(request_id);
    }

    /// Distinguishes a live call, a known timed-out call, and a genuinely unknown response id.
    pub(crate) fn take_response(&mut self, request_id: u64) -> ResponseRequest {
        if let Some(sender) = self.active.remove(&request_id) {
            ResponseRequest::Pending(sender)
        } else if self.abandoned.contains(&request_id) {
            ResponseRequest::Abandoned
        } else {
            ResponseRequest::Unmatched
        }
    }

    /// Releases every live caller and forgets tombstones when the process generation ends.
    pub(crate) fn clear(&mut self) -> HashMap<u64, oneshot::Sender<PendingResult>> {
        self.abandoned.clear();
        std::mem::take(&mut self.active)
    }
}

/// Tracks the single lifecycle a plugin connection can be in at any moment.
///
/// `Failed` and `ShuttingDown` are distinct because only the latter is expected: it suppresses
/// the restart and error reporting that an unexpected failure must trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeStatus {
    Starting,
    Ready,
    Failed(String),
    ShuttingDown,
}

/// Commands the process supervisor accepts from the protocol tasks and the public handle.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SupervisorCommand {
    Shutdown,
    ProtocolFailure,
}

/// Holds the state every protocol task shares for one launched plugin process.
pub(crate) struct RuntimeInner {
    pub plugin_id: String,
    pub registration: RwLock<PluginRegistration>,
    pub status_tx: watch::Sender<RuntimeStatus>,
    /// Flips to `true` once the supervisor confirms the child process tree has fully exited.
    pub exited_tx: watch::Sender<bool>,
    pub writer_tx: mpsc::Sender<Value>,
    pub supervisor_tx: mpsc::UnboundedSender<SupervisorCommand>,
    pub inbound: Mutex<Option<mpsc::UnboundedSender<PluginNotification>>>,
    pub pending: Mutex<PendingRequests>,
    pub next_request_id: AtomicU64,
    pub call_timeout: Duration,
}

/// Marks a protocol connection unusable and wakes every waiting caller.
pub(crate) async fn fail_runtime(inner: &Arc<RuntimeInner>, reason: String) {
    inner
        .status_tx
        .send_replace(RuntimeStatus::Failed(reason.clone()));
    fail_pending(inner, PluginRuntimeError::Unavailable(reason)).await;
    close_inbound(inner).await;
    let _ = inner.supervisor_tx.send(SupervisorCommand::ProtocolFailure);
}

/// Closes the plugin-originated notification stream so upper layers observe process loss.
pub(crate) async fn close_inbound(inner: &RuntimeInner) {
    inner.inbound.lock().await.take();
}

/// Completes all pending requests with the same terminal runtime failure.
pub(crate) async fn fail_pending(inner: &RuntimeInner, error: PluginRuntimeError) {
    let pending = inner.pending.lock().await.clear();
    for sender in pending.into_values() {
        let _ = sender.send(Err(error.clone()));
    }
}

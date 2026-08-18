use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};

use crate::PluginRuntimeError;
use crate::protocol::{PluginNotification, PluginRegistration};

pub(crate) type PendingResult = Result<Value, PluginRuntimeError>;

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
    pub inbound: mpsc::UnboundedSender<PluginNotification>,
    pub pending: Mutex<HashMap<u64, oneshot::Sender<PendingResult>>>,
    pub next_request_id: AtomicU64,
    pub call_timeout: Duration,
}

/// Marks a protocol connection unusable and wakes every waiting caller.
pub(crate) async fn fail_runtime(inner: &Arc<RuntimeInner>, reason: String) {
    inner
        .status_tx
        .send_replace(RuntimeStatus::Failed(reason.clone()));
    fail_pending(inner, PluginRuntimeError::Unavailable(reason)).await;
    let _ = inner.supervisor_tx.send(SupervisorCommand::ProtocolFailure);
}

/// Completes all pending requests with the same terminal runtime failure.
pub(crate) async fn fail_pending(inner: &RuntimeInner, error: PluginRuntimeError) {
    let pending = std::mem::take(&mut *inner.pending.lock().await);
    for sender in pending.into_values() {
        let _ = sender.send(Err(error.clone()));
    }
}

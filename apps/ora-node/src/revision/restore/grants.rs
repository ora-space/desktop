//! Download grants for prior bundles, held only in memory, and the requests for them.
//!
//! Like an upload grant, a download grant is a bearer credential that expires, so it never enters
//! the ledger, a log line or a `Debug` print. The control session hands every inbound
//! `DownloadGrant` to [`DownloadGrants::offer`] without queueing it behind admission, forwards
//! every [`DownloadGrantNeededMessage`] the store emits, and sends [`DownloadGrants::pending`]
//! first on every new connection, because the Controller remembers no request across connections.
use ora_node_protocol::{
    CURRENT_PROTOCOL_VERSION, DownloadGrant, DownloadGrantMessage, DownloadGrantNeeded,
    DownloadGrantNeededMessage, ExecutionId, NodeId, ObjectDownloadGrant, ObjectKey, OperationId,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// Requests not yet written by a slow session are superseded by the next periodic request.
const REQUEST_BUFFER: usize = 16;

/// Download grants and grant requests shared by restoring sessions and the control session.
#[derive(Clone)]
pub(crate) struct DownloadGrants {
    inner: Arc<Inner>,
}

struct Inner {
    waiting: Mutex<HashMap<ExecutionId, Waiting>>,
    /// Bumped on every accepted answer so a restore blocked on a missing grant wakes up.
    offered: watch::Sender<u64>,
    requests: broadcast::Sender<DownloadGrantNeededMessage>,
}

/// A restoring session: the one object it reads and the Controller's latest answer for it.
struct Waiting {
    operation: OperationId,
    node: NodeId,
    key: ObjectKey,
    answer: Option<Answer>,
    /// Set while the restore waits for an answer, so a new connection can repeat the request.
    requesting: bool,
}

/// What the Controller said about one restore's download.
#[derive(Clone)]
pub(crate) enum Answer {
    Granted(ObjectDownloadGrant),
    /// Cloud will not grant the read; the restore fails.
    Refused,
}

impl DownloadGrants {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                waiting: Mutex::new(HashMap::new()),
                offered: watch::channel(/*init*/ 0).0,
                requests: broadcast::channel(REQUEST_BUFFER).0,
            }),
        }
    }

    /// Delivers grant requests to one connection; requests sent while nobody listens are lost,
    /// which is harmless because a waiting restore repeats its request.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<DownloadGrantNeededMessage> {
        self.inner.requests.subscribe()
    }

    /// Keeps a grant only for the object the registered restore reads, and a refusal only for a
    /// registered restore: anything else could make the Node read something it was not asked to.
    pub(crate) fn offer(&self, message: DownloadGrantMessage) {
        let mut waiting = self.lock();
        let Some(restore) = waiting.get_mut(&message.execution_id) else {
            ora_logging::ora_debug!(execution_id = %message.execution_id.as_str(), "ignored a download grant for an execution that is not restoring");
            return;
        };
        if restore.operation != message.operation_id {
            ora_logging::ora_warn!(execution_id = %message.execution_id.as_str(), "ignored a download grant naming another operation");
            return;
        }
        match message.payload {
            DownloadGrant::Granted { grants } => {
                for grant in grants {
                    if grant.object_key == restore.key {
                        restore.answer = Some(Answer::Granted(grant));
                    } else {
                        ora_logging::ora_warn!(execution_id = %message.execution_id.as_str(), object_key = %grant.object_key.as_str(), "ignored a download grant for an object the restore does not read");
                    }
                }
            }
            DownloadGrant::Refused {} => restore.answer = Some(Answer::Refused),
        }
        drop(waiting);
        self.inner
            .offered
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Starts accepting answers for `key`; dropping the registration forgets them and any grant.
    pub(crate) fn register(
        &self,
        operation: OperationId,
        execution: ExecutionId,
        node: NodeId,
        key: ObjectKey,
    ) -> DownloadRegistration {
        self.lock().insert(
            execution.clone(),
            Waiting {
                operation,
                node,
                key,
                answer: None,
                requesting: false,
            },
        );
        DownloadRegistration {
            store: self.clone(),
            execution,
        }
    }

    /// The requests of every restore currently waiting for an answer, which a new Controller
    /// connection sends first.
    pub(crate) fn pending(&self) -> Vec<DownloadGrantNeededMessage> {
        self.lock()
            .iter()
            .filter(|(_, waiting)| waiting.requesting)
            .map(|(execution, waiting)| waiting.request(execution))
            .collect()
    }

    /// Observes answer arrivals; mark the receiver seen before checking for an answer.
    pub(crate) fn offers(&self) -> watch::Receiver<u64> {
        self.inner.offered.subscribe()
    }

    /// Mutex poisoning only means another thread panicked mid-update of a plain map; grants are
    /// re-requestable, so the data is still safe to use.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ExecutionId, Waiting>> {
        self.inner
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// One restore's live claim on the store.
pub(crate) struct DownloadRegistration {
    store: DownloadGrants,
    execution: ExecutionId,
}

impl DownloadRegistration {
    /// The current answer: a refusal, or a grant valid for at least `margin`. An expired grant is
    /// dropped, so the next request asks for a fresh one.
    pub(crate) fn answer(&self, margin: Duration) -> Option<Answer> {
        let mut waiting = self.store.lock();
        let restore = waiting.get_mut(&self.execution)?;
        let answer = match restore.answer.take()? {
            Answer::Granted(grant)
                if grant.expires_at <= ora_logging::clock::now_local() + margin =>
            {
                return None;
            }
            answer @ (Answer::Granted(_) | Answer::Refused) => answer,
        };
        restore.requesting = false;
        restore.answer = Some(answer.clone());
        Some(answer)
    }

    /// Forgets a grant the object store refused, so the next attempt asks for a fresh one.
    pub(crate) fn discard(&self) {
        if let Some(restore) = self.store.lock().get_mut(&self.execution) {
            restore.answer = None;
        }
    }

    /// Asks the Controller for a grant of the prior bundle.
    pub(crate) fn request(&self) {
        let message = {
            let mut waiting = self.store.lock();
            let Some(restore) = waiting.get_mut(&self.execution) else {
                return;
            };
            restore.requesting = true;
            restore.request(&self.execution)
        };
        // No connected session is not an error: the request is repeated while the restore waits.
        let _ = self.store.inner.requests.send(message);
    }
}

impl Drop for DownloadRegistration {
    /// A finished or abandoned restore must not keep credentials in memory.
    fn drop(&mut self) {
        self.store.lock().remove(&self.execution);
    }
}

impl Waiting {
    /// The grant request for this restore's object.
    fn request(&self, execution: &ExecutionId) -> DownloadGrantNeededMessage {
        DownloadGrantNeededMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: self.operation.clone(),
            execution_id: execution.clone(),
            payload: DownloadGrantNeeded {
                node_id: self.node.clone(),
            },
        }
    }
}

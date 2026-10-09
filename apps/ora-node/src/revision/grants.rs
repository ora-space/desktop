//! Upload grants held only in memory, and the requests for them.
//!
//! A grant is a bearer credential that expires, so it never enters the ledger, a log line or a
//! `Debug` print: a restarted Node asks again instead (protocol D5). The control session hands
//! every inbound `UploadGrant` to [`GrantStore::offer`] without queueing it behind admission, and
//! forwards every [`UploadGrantNeededMessage`] the store emits to the connected Controller.
use ora_node_protocol::{
    CURRENT_PROTOCOL_VERSION, ExecutionId, NodeId, ObjectKey, ObjectUploadGrant, OperationId,
    Sha256Digest, UploadGrantMessage, UploadGrantNeeded, UploadGrantNeededMessage,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// Requests not yet written by a slow session are superseded by the next periodic request.
const REQUEST_BUFFER: usize = 16;

/// Grants and grant requests shared by the delivery executor and the control session.
#[derive(Clone)]
pub(crate) struct GrantStore {
    inner: Arc<Inner>,
}

struct Inner {
    waiting: Mutex<HashMap<ExecutionId, Waiting>>,
    /// Bumped on every accepted grant so an uploader blocked on a missing grant wakes up.
    offered: watch::Sender<u64>,
    requests: broadcast::Sender<UploadGrantNeededMessage>,
}

/// An uploading execution: the keys it still needs and the grants it holds for them.
struct Waiting {
    operation: OperationId,
    node: NodeId,
    keys: BTreeMap<ObjectKey, Sha256Digest>,
    grants: HashMap<ObjectKey, ObjectUploadGrant>,
    /// Set while the upload waits for a grant, so a new connection can repeat the request at once.
    requesting: bool,
}

impl GrantStore {
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
    /// which is harmless because a waiting upload repeats its request.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<UploadGrantNeededMessage> {
        self.inner.requests.subscribe()
    }

    /// Keeps the grants for keys the execution is currently uploading and drops the rest: a grant
    /// for an unknown execution or key could only make the Node write somewhere it was not asked to.
    pub(crate) fn offer(&self, message: UploadGrantMessage) {
        let mut waiting = self.lock();
        let Some(execution) = waiting.get_mut(&message.execution_id) else {
            ora_logging::ora_debug!(execution_id = %message.execution_id.as_str(), "ignored upload grants for an execution that is not uploading");
            return;
        };
        if execution.operation != message.operation_id {
            ora_logging::ora_warn!(execution_id = %message.execution_id.as_str(), "ignored upload grants naming another operation");
            return;
        }
        for grant in message.payload.grants {
            if execution.keys.contains_key(&grant.object_key) {
                execution.grants.insert(grant.object_key.clone(), grant);
            } else {
                ora_logging::ora_warn!(execution_id = %message.execution_id.as_str(), object_key = %grant.object_key.as_str(), "ignored an upload grant for a key the delivery does not upload");
            }
        }
        drop(waiting);
        self.inner
            .offered
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Starts accepting grants for `keys`; dropping the registration forgets them and any grant.
    pub(crate) fn register(
        &self,
        operation: OperationId,
        execution: ExecutionId,
        node: NodeId,
        keys: BTreeMap<ObjectKey, Sha256Digest>,
    ) -> Registration {
        self.lock().insert(
            execution.clone(),
            Waiting {
                operation,
                node,
                keys,
                grants: HashMap::new(),
                requesting: false,
            },
        );
        Registration {
            store: self.clone(),
            execution,
        }
    }

    /// The requests of every upload currently waiting for a grant. A new Controller connection
    /// sends these first: a restarted Controller remembers no earlier request.
    pub(crate) fn pending(&self) -> Vec<UploadGrantNeededMessage> {
        self.lock()
            .iter()
            .filter(|(_, waiting)| waiting.requesting && !waiting.keys.is_empty())
            .map(|(execution, waiting)| waiting.request(execution))
            .collect()
    }

    /// Observes grant arrivals; mark the receiver seen before checking for a grant.
    pub(crate) fn offers(&self) -> watch::Receiver<u64> {
        self.inner.offered.subscribe()
    }

    /// Mutex poisoning only means another thread panicked mid-update of plain maps; grants are
    /// re-requestable, so the data is still safe to use.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ExecutionId, Waiting>> {
        self.inner
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// One execution's live claim on the store.
pub(crate) struct Registration {
    store: GrantStore,
    execution: ExecutionId,
}

impl Registration {
    /// Returns a grant for `key` that stays valid for at least `margin`, dropping an expired one.
    pub(crate) fn valid(&self, key: &ObjectKey, margin: Duration) -> Option<ObjectUploadGrant> {
        let mut waiting = self.store.lock();
        let execution = waiting.get_mut(&self.execution)?;
        let usable = execution
            .grants
            .get(key)
            .is_some_and(|grant| grant.expires_at > ora_logging::clock::now_local() + margin);
        if usable {
            execution.requesting = false;
            return execution.grants.get(key).cloned();
        }
        execution.grants.remove(key);
        None
    }

    /// Forgets a grant the store refused, so the next attempt asks for a fresh one.
    pub(crate) fn discard(&self, key: &ObjectKey) {
        if let Some(execution) = self.store.lock().get_mut(&self.execution) {
            execution.grants.remove(key);
        }
    }

    /// Marks an object uploaded: later requests no longer ask for it, and its grant is dropped.
    pub(crate) fn uploaded(&self, key: &ObjectKey) {
        if let Some(execution) = self.store.lock().get_mut(&self.execution) {
            execution.keys.remove(key);
            execution.grants.remove(key);
        }
    }

    /// Asks the Controller for grants covering every object still to upload, with the digests
    /// frozen before the first PUT so Cloud can bind each grant to exactly those bytes.
    pub(crate) fn request(&self) {
        let message = {
            let mut waiting = self.store.lock();
            let Some(execution) = waiting.get_mut(&self.execution) else {
                return;
            };
            if execution.keys.is_empty() {
                return;
            }
            execution.requesting = true;
            execution.request(&self.execution)
        };
        // No connected session is not an error: the request is repeated while the upload waits.
        let _ = self.store.inner.requests.send(message);
    }
}

impl Drop for Registration {
    /// A finished or interrupted upload must not keep credentials in memory.
    fn drop(&mut self) {
        self.store.lock().remove(&self.execution);
    }
}

impl Waiting {
    /// The grant request for everything this upload still has to store.
    fn request(&self, execution: &ExecutionId) -> UploadGrantNeededMessage {
        UploadGrantNeededMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: self.operation.clone(),
            execution_id: execution.clone(),
            payload: UploadGrantNeeded {
                node_id: self.node.clone(),
                checksums: self.keys.clone(),
            },
        }
    }
}

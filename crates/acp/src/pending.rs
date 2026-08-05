use ora_contracts::acp::common::SessionId;
use ora_contracts::acp::error::Error as RpcError;
use ora_contracts::acp::rpc::RequestId;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use tokio::sync::oneshot;

// Bound per-connection stale-id memory; responses older than this window are protocol failures.
const ABANDONED_REQUEST_CAPACITY: usize = 256;

pub(super) type PendingResponse = Result<Value, RpcError>;

pub(super) enum PendingRequest {
    Direct(oneshot::Sender<PendingResponse>),
    Session { session_id: SessionId },
}

pub(super) enum ResponseRequest {
    Pending(PendingRequest),
    Abandoned,
    Unmatched,
}

/// Keeps active correlation entries separate from bounded abandoned-request tombstones.
#[derive(Default)]
pub(super) struct PendingRequests {
    active: HashMap<RequestId, PendingRequest>,
    abandoned: VecDeque<RequestId>,
}

impl PendingRequests {
    /// Registers one request until its response is observed or the caller abandons it.
    pub(super) fn insert(&mut self, request_id: RequestId, request: PendingRequest) {
        self.active.insert(request_id, request);
    }

    /// Removes a request that could not be written and therefore cannot produce a valid response.
    pub(super) fn remove_active(&mut self, request_id: &RequestId) {
        self.active.remove(request_id);
    }

    /// Retires a request while retaining enough identity to recognize its late response.
    pub(super) fn abandon(&mut self, request_id: &RequestId) {
        if self.active.remove(request_id).is_none() {
            return;
        }
        if self.abandoned.len() == ABANDONED_REQUEST_CAPACITY {
            self.abandoned.pop_front();
        }
        self.abandoned.push_back(request_id.clone());
    }

    /// Distinguishes active, explicitly abandoned, and genuinely unknown response ids.
    pub(super) fn take_response(&mut self, request_id: &RequestId) -> ResponseRequest {
        if let Some(request) = self.active.remove(request_id) {
            ResponseRequest::Pending(request)
        } else if self.abandoned.contains(request_id) {
            ResponseRequest::Abandoned
        } else {
            ResponseRequest::Unmatched
        }
    }

    /// Releases active senders and retired ids when the connection generation ends.
    pub(super) fn clear(&mut self) {
        self.active.clear();
        self.abandoned.clear();
    }
}

/// Recovers a poisoned pending registry so correlation can continue after a panic.
pub(super) fn lock_pending(
    pending: &Mutex<PendingRequests>,
) -> std::sync::MutexGuard<'_, PendingRequests> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

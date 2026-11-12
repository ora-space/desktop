//! Host-issued attempt tokens and the accepted agent completion signals they carry.
//!
//! The completion tool is exposed per prompt attempt: `issue` mints a token, the agent passes it
//! back through the MCP tool, and `accept` records the request against that attempt. The park site
//! later calls `bind` with the settled turn so a late request can never attach to a newer turn.
//!
//! The store is intentionally in memory: a token lives for one attempt, and an Ora restart ends
//! the prompt it belonged to. Nothing here is durable; only the bound `AgentSignal` is persisted in
//! the node payload.
#![allow(dead_code)] // Wired into the session/MCP path in the next step of #1070.

use crate::workflow::run::completion_snapshot::{AgentSignal, TurnKey};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// One issued attempt token and the request it accepted, if any.
#[derive(Debug, Clone)]
struct TokenRecord {
    node_run_id: String,
    attempt_id: String,
    signal_id: Option<String>,
}

/// One accepted request that has not been bound to a settled turn yet.
struct PendingSignal {
    signal_id: String,
    requested_at: i64,
}

/// Why an attempt token could not be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignalError {
    /// The token was never issued, was revoked, or already belonged to a finished node.
    UnknownToken,
}

/// Per-backend store for attempt tokens and pending completion requests.
#[derive(Default)]
pub(crate) struct AgentSignalStore {
    tokens: Mutex<HashMap<String, TokenRecord>>,
    pending: Mutex<HashMap<(String, String), PendingSignal>>,
}

impl AgentSignalStore {
    /// Creates an empty store; one instance is shared by the backend composition.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Issues one token for a prompt attempt. The token is the only capability the MCP tool needs.
    pub(crate) fn issue(&self, node_run_id: &str, attempt_id: &str) -> String {
        let token = Uuid::new_v4().to_string();
        self.tokens.lock().expect("agent signal tokens").insert(
            token.clone(),
            TokenRecord {
                node_run_id: node_run_id.to_string(),
                attempt_id: attempt_id.to_string(),
                signal_id: None,
            },
        );
        token
    }

    /// Accepts the tool call for an attempt token, idempotently returning the same signal id.
    pub(crate) fn accept(&self, token: &str, requested_at: i64) -> Result<String, SignalError> {
        let mut tokens = self.tokens.lock().expect("agent signal tokens");
        let record = tokens.get_mut(token).ok_or(SignalError::UnknownToken)?;
        if let Some(signal_id) = &record.signal_id {
            return Ok(signal_id.clone());
        }
        let signal_id = Uuid::new_v4().to_string();
        record.signal_id = Some(signal_id.clone());
        self.pending
            .lock()
            .expect("agent signal pending")
            .insert(
                (record.node_run_id.clone(), record.attempt_id.clone()),
                PendingSignal {
                    signal_id: signal_id.clone(),
                    requested_at,
                },
            );
        Ok(signal_id)
    }

    /// Binds a pending request to the turn that attempt settled, if one was accepted.
    ///
    /// A request from an attempt that produced no settled turn returns `None`; the attempt's token
    /// and pending entry are dropped by [`Self::revoke_attempt`] or the next park.
    pub(crate) fn bind(
        &self,
        node_run_id: &str,
        attempt_id: &str,
        turn_key: TurnKey,
    ) -> Option<AgentSignal> {
        let pending = self
            .pending
            .lock()
            .expect("agent signal pending")
            .remove(&(node_run_id.to_string(), attempt_id.to_string()))?;
        Some(AgentSignal {
            signal_id: pending.signal_id,
            requested_at: pending.requested_at,
            turn_key,
        })
    }

    /// Drops one attempt's token and pending request when the prompt ends without parking.
    pub(crate) fn revoke_attempt(&self, node_run_id: &str, attempt_id: &str) {
        self.tokens
            .lock()
            .expect("agent signal tokens")
            .retain(|_, record| {
                !(record.node_run_id == node_run_id && record.attempt_id == attempt_id)
            });
        self.pending
            .lock()
            .expect("agent signal pending")
            .remove(&(node_run_id.to_string(), attempt_id.to_string()));
    }

    /// Drops every token and pending request for a node that left `Pending`/`Running`.
    pub(crate) fn revoke_node(&self, node_run_id: &str) {
        self.tokens
            .lock()
            .expect("agent signal tokens")
            .retain(|_, record| record.node_run_id != node_run_id);
        self.pending
            .lock()
            .expect("agent signal pending")
            .retain(|(node, _), _| node != node_run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentSignalStore, SignalError};
    use crate::workflow::run::completion_snapshot::TurnKey;
    use pretty_assertions::assert_eq;

    fn turn(user_seq: u32, ended_seq: u32) -> TurnKey {
        TurnKey {
            user_seq,
            ended_seq,
        }
    }

    #[test]
    fn an_issued_token_accepts_once_and_binds_to_its_own_attempt() {
        let store = AgentSignalStore::new();
        let token = store.issue("node-1", "attempt-1");
        let signal_id = store.accept(&token, 10).expect("accepted");
        let duplicate = store.accept(&token, 11).expect("idempotent");
        assert_eq!(signal_id, duplicate);

        let bound = store
            .bind("node-1", "attempt-1", turn(1, 2))
            .expect("bound");
        assert_eq!(bound.signal_id, signal_id);
        assert_eq!(bound.requested_at, 10);
        assert_eq!(bound.turn_key, turn(1, 2));
        assert!(store.bind("node-1", "attempt-2", turn(1, 2)).is_none());
    }

    #[test]
    fn unknown_and_revoked_tokens_are_rejected() {
        let store = AgentSignalStore::new();
        assert_eq!(
            store.accept("missing", 1),
            Err(SignalError::UnknownToken)
        );
        let token = store.issue("node-1", "attempt-1");
        store.revoke_attempt("node-1", "attempt-1");
        assert_eq!(store.accept(&token, 1), Err(SignalError::UnknownToken));
    }

    #[test]
    fn a_request_never_binds_to_another_nodes_turn() {
        let store = AgentSignalStore::new();
        let token = store.issue("node-1", "attempt-1");
        store.accept(&token, 5).expect("accepted");
        assert!(store.bind("node-2", "attempt-1", turn(1, 2)).is_none());
        store.revoke_node("node-1");
        assert!(store.bind("node-1", "attempt-1", turn(1, 2)).is_none());
    }
}

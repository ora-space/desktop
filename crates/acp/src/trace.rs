use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// Keeps one provider session associated with the Ora session shown in logs.
///
/// Dropping the registration removes only the association it created, so a stale
/// session channel cannot erase a newer binding for the same provider identifier.
pub struct SessionTraceRegistration {
    agent_session_id: String,
    token: u64,
    registry: SessionTraceRegistry,
}

#[derive(Clone, Default)]
pub(super) struct SessionTraceRegistry {
    entries: Arc<RwLock<HashMap<String, TraceEntry>>>,
    next_token: Arc<AtomicU64>,
}

struct TraceEntry {
    ora_session_id: String,
    token: u64,
}

impl SessionTraceRegistry {
    /// Associates transport traffic with the Ora-owned session identifier.
    pub(super) fn register(
        &self,
        agent_session_id: &str,
        ora_session_id: &str,
    ) -> SessionTraceRegistration {
        let token = self.next_token.fetch_add(1, Ordering::Relaxed) + 1;
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                agent_session_id.to_string(),
                TraceEntry {
                    ora_session_id: ora_session_id.to_string(),
                    token,
                },
            );
        SessionTraceRegistration {
            agent_session_id: agent_session_id.to_string(),
            token,
            registry: self.clone(),
        }
    }

    /// Resolves the application identity without exposing the mutable registry.
    pub(super) fn resolve(&self, agent_session_id: &str) -> String {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent_session_id)
            .map(|entry| entry.ora_session_id.clone())
            .unwrap_or_default()
    }
}

impl Drop for SessionTraceRegistration {
    fn drop(&mut self) {
        let mut entries = self
            .registry
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if entries
            .get(&self.agent_session_id)
            .is_some_and(|entry| entry.token == self.token)
        {
            entries.remove(&self.agent_session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionTraceRegistry;
    use pretty_assertions::assert_eq;

    /// Verifies a stale registration cannot erase a replacement association.
    #[test]
    fn preserves_replacement_when_stale_registration_drops() {
        let registry = SessionTraceRegistry::default();
        let stale = registry.register("agent-session", "ora-session-1");
        let current = registry.register("agent-session", "ora-session-2");

        drop(stale);

        assert_eq!(registry.resolve("agent-session"), "ora-session-2");
        drop(current);
        assert_eq!(registry.resolve("agent-session"), "");
    }
}

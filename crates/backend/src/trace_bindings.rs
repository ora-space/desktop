//! Binds workbench surface instances to the agent session they were opened for.
//!
//! `TraceHost` uses this table to turn a page's surface identity into the one session the page
//! may read. The table is keyed by surface instance id only: the surface generation is pinned to
//! the serving process *before* the lookup (in `TraceHost`), so storing it here as well would be
//! a second, driftable copy.

use crate::trace_host::{TraceSessionBinding, TraceSessionBindingLookup};
use std::collections::HashMap;
use std::sync::Mutex;

/// The runtime binding table: written when a session-bound panel opens, cleared entry-by-entry
/// when one closes.
#[derive(Debug, Default)]
pub struct TraceBindingRegistry {
    bindings: Mutex<HashMap<u64, TraceSessionBinding>>,
}

impl TraceBindingRegistry {
    /// Builds an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces one surface's binding.
    pub fn register(&self, instance_id: u64, binding: TraceSessionBinding) {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(instance_id, binding);
    }

    /// Removes one surface's binding; idempotent when none exists.
    pub fn unregister(&self, instance_id: u64) {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&instance_id);
    }

    /// Drops every binding (host shutdown / surface teardown).
    pub fn clear_all(&self) {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

impl TraceSessionBindingLookup for TraceBindingRegistry {
    /// Returns the binding for one surface, or `None` when it has none.
    ///
    /// `generation` is accepted for interface parity but deliberately ignored: `TraceHost`
    /// validates it against the serving process before calling here.
    fn binding(&self, instance_id: u64, _generation: u64) -> Option<TraceSessionBinding> {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&instance_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_domain::AgentRef;

    /// Extracts an expected successful result without using `expect` in tests.
    fn must<T, E: std::fmt::Debug>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected {label} to succeed, got {error:?}"),
        }
    }

    fn binding_for(agent: &str, session_id: &str) -> TraceSessionBinding {
        TraceSessionBinding {
            agent_ref: must(AgentRef::parse(agent), "agent ref"),
            agent_session_id: session_id.to_owned(),
        }
    }

    /// Register, replace, and unregister keep the table consistent.
    #[test]
    fn register_replace_and_unregister() {
        let table = TraceBindingRegistry::new();
        table.register(7, binding_for("ora-space.claude", "abc-123"));
        table.register(8, binding_for("ora-space.opencode", "ses_1"));

        let lookup = &table as &dyn TraceSessionBindingLookup;
        assert_eq!(
            lookup.binding(7, 1).map(|binding| binding.agent_session_id),
            Some("abc-123".to_owned()),
        );
        // The generation is ignored here: TraceHost validates it before this lookup.
        assert_eq!(
            lookup
                .binding(7, 99)
                .map(|binding| binding.agent_session_id),
            Some("abc-123".to_owned()),
        );
        assert_eq!(lookup.binding(9, 1), None);

        // A reopened surface replaces the binding, never stacks a second one.
        table.register(7, binding_for("ora-space.opencode", "ses_2"));
        assert_eq!(
            lookup.binding(7, 1).map(|binding| binding.agent_session_id),
            Some("ses_2".to_owned()),
        );

        table.unregister(7);
        assert_eq!(lookup.binding(7, 1), None);
        table.unregister(7); // idempotent
        assert_eq!(
            lookup.binding(8, 1).map(|binding| binding.agent_session_id),
            Some("ses_1".to_owned())
        );

        table.clear_all();
        assert_eq!(lookup.binding(8, 1), None);
    }
}

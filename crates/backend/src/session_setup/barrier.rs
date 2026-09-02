//! Shared Agent Session Barrier for MCP refresh, Effect mutation, and Agent replacement.
//!
//! The barrier is the coordination seam only. MCP does not publish Effect state, and Effect
//! Target readiness does not prove any Session has loaded MCP.

use ora_domain::PluginId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Why one Agent's sessions must stop admitting new prompts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BarrierReason {
    McpRefresh,
    EffectMutation,
    AgentReplacement,
}

/// One Agent's mutex that serializes MCP refresh with Effect mutation and process replacement.
#[derive(Debug, Default)]
pub(crate) struct AgentSessionBarrier {
    lock: Arc<Mutex<()>>,
}

/// Holds the barrier until the owning lifecycle step finishes.
pub(crate) struct BarrierGuard {
    reason: BarrierReason,
    _guard: OwnedMutexGuard<()>,
}

impl BarrierGuard {
    /// Reason recorded when this hold was acquired.
    pub(crate) fn reason(&self) -> BarrierReason {
        self.reason
    }
}

impl AgentSessionBarrier {
    /// Waits until no other lifecycle step holds the Agent, then records `reason`.
    pub(crate) async fn acquire(&self, reason: BarrierReason) -> BarrierGuard {
        BarrierGuard {
            reason,
            _guard: self.lock.clone().lock_owned().await,
        }
    }

    /// Returns a hold only when the Agent is not already fenced.
    pub(crate) fn try_acquire(&self, reason: BarrierReason) -> Option<BarrierGuard> {
        Some(BarrierGuard {
            reason,
            _guard: self.lock.clone().try_lock_owned().ok()?,
        })
    }

    /// Whether another lifecycle step currently owns the Agent.
    pub(crate) fn is_held(&self) -> bool {
        self.lock.try_lock().is_err()
    }
}

/// Lazily allocates one barrier per Agent plugin so unrelated Agents stay concurrent.
#[derive(Debug, Default)]
pub(crate) struct AgentSessionBarriers {
    by_plugin: std::sync::Mutex<HashMap<PluginId, Arc<AgentSessionBarrier>>>,
}

impl AgentSessionBarriers {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the barrier for one Agent plugin, creating it on first use.
    pub(crate) fn for_plugin(&self, plugin_id: &PluginId) -> Arc<AgentSessionBarrier> {
        let mut barriers = self
            .by_plugin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        barriers.entry(plugin_id.clone()).or_default().clone()
    }
}

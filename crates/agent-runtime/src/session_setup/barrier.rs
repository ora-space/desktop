//! Agent Session Barrier for live MCP refresh.
//!
//! Serializes per-Agent `session/load` MCP refreshes so concurrent sessions targeting the same
//! Agent plugin do not race each other during setup/refresh.

use ora_domain::PluginId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Why one Agent's sessions hold the shared session barrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarrierReason {
    McpRefresh,
}

/// One Agent's mutex that serializes MCP refresh operations for sessions sharing that Agent.
#[derive(Debug, Default)]
pub struct AgentSessionBarrier {
    lock: Arc<Mutex<()>>,
}

/// Holds the barrier until the owning lifecycle step finishes.
pub struct BarrierGuard {
    reason: BarrierReason,
    _guard: OwnedMutexGuard<()>,
}

impl BarrierGuard {
    /// Reason recorded when this hold was acquired.
    pub fn reason(&self) -> BarrierReason {
        self.reason
    }
}

impl AgentSessionBarrier {
    /// Waits until no other lifecycle step holds the Agent, then records `reason`.
    pub async fn acquire(&self, reason: BarrierReason) -> BarrierGuard {
        BarrierGuard {
            reason,
            _guard: self.lock.clone().lock_owned().await,
        }
    }

    /// Bounds the wait without granting authority when another operation owns the lock.
    pub async fn acquire_with_timeout(
        &self,
        reason: BarrierReason,
        timeout: Duration,
    ) -> Result<BarrierGuard, tokio::time::error::Elapsed> {
        tokio::time::timeout(timeout, self.acquire(reason)).await
    }

    /// Returns a hold only when the Agent is not already fenced.
    pub fn try_acquire(&self, reason: BarrierReason) -> Option<BarrierGuard> {
        Some(BarrierGuard {
            reason,
            _guard: self.lock.clone().try_lock_owned().ok()?,
        })
    }

    /// Whether another lifecycle step currently owns the Agent.
    pub fn is_held(&self) -> bool {
        self.lock.try_lock().is_err()
    }
}

/// Lazily allocates one barrier per Agent plugin so unrelated Agents stay concurrent.
#[derive(Debug, Default)]
pub struct AgentSessionBarriers {
    by_plugin: std::sync::Mutex<HashMap<PluginId, Arc<AgentSessionBarrier>>>,
}

impl AgentSessionBarriers {
    /// Creates an empty per-plugin barrier registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the barrier for one Agent plugin, creating it on first use.
    pub fn for_plugin(&self, plugin_id: &PluginId) -> Arc<AgentSessionBarrier> {
        let mut barriers = self
            .by_plugin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        barriers.entry(plugin_id.clone()).or_default().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentSessionBarriers, BarrierReason};
    use ora_domain::PluginId;

    /// Names one Agent plugin under the `ora-space` namespace.
    fn plugin(name: &str) -> PluginId {
        PluginId::new("ora-space", name).expect("plugin id")
    }

    #[tokio::test]
    async fn mcp_refresh_holds_one_agent_session_barrier_per_agent() {
        let barriers = AgentSessionBarriers::new();
        let plugin = plugin("opencode");
        let first = barriers
            .for_plugin(&plugin)
            .try_acquire(BarrierReason::McpRefresh)
            .expect("first hold");
        assert!(barriers.for_plugin(&plugin).is_held());
        assert!(
            barriers
                .for_plugin(&plugin)
                .try_acquire(BarrierReason::McpRefresh)
                .is_none()
        );
        drop(first);
        assert!(
            barriers
                .for_plugin(&plugin)
                .try_acquire(BarrierReason::McpRefresh)
                .is_some()
        );
    }

    /// A timed-out waiter leaves the existing guard intact and can retry after release.
    #[tokio::test(start_paused = true)]
    async fn timed_out_acquisition_does_not_steal_the_barrier() {
        let barriers = AgentSessionBarriers::new();
        let barrier = barriers.for_plugin(&plugin("opencode"));
        let owner = barrier.acquire(BarrierReason::McpRefresh).await;
        let timeout = std::time::Duration::from_secs(5);
        assert!(
            barrier
                .acquire_with_timeout(BarrierReason::McpRefresh, timeout)
                .await
                .is_err()
        );
        assert!(barrier.is_held());
        let unrelated = barriers.for_plugin(&plugin("claude"));
        assert!(
            unrelated
                .acquire_with_timeout(BarrierReason::McpRefresh, timeout)
                .await
                .is_ok()
        );
        drop(owner);
        assert!(
            barrier
                .acquire_with_timeout(BarrierReason::McpRefresh, timeout)
                .await
                .is_ok()
        );
    }

    #[test]
    fn unrelated_agents_do_not_share_a_barrier() {
        let barriers = AgentSessionBarriers::new();
        let _hold = barriers
            .for_plugin(&plugin("opencode"))
            .try_acquire(BarrierReason::McpRefresh)
            .expect("opencode hold");
        assert!(
            barriers
                .for_plugin(&plugin("claude"))
                .try_acquire(BarrierReason::McpRefresh)
                .is_some()
        );
    }
}

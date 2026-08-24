//! Idle stop of plugin processes once their last surface instance has closed.
//!
//! The timers are plain cancellation tokens: the service arms one when a plugin's instance
//! count drops to zero and disarms it on the next open. Waiting is a separate pure-async
//! function so tests can drive it with paused time and any spawner.

use ora_domain::PluginId;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// How long a plugin may have zero instances before its process is stopped.
pub const IDLE_GRACE: Duration = Duration::from_secs(30);

/// Result of one idle wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleOutcome {
    /// The grace period elapsed without a reopen; the caller re-checks the instance count.
    Expired,
    /// A surface of the plugin was reopened meanwhile.
    Cancelled,
}

/// One armed token per plugin with an idle grace period running.
#[derive(Debug, Default)]
pub struct IdleTimers {
    armed: Mutex<HashMap<PluginId, CancellationToken>>,
}

impl IdleTimers {
    /// Starts (or restarts) the grace period for `plugin_id` and returns the token to wait on.
    ///
    /// Re-arming cancels the previous token so at most one stop can fire per idle period.
    pub fn arm(&self, plugin_id: &PluginId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(previous) = self.lock().insert(plugin_id.clone(), token.clone()) {
            previous.cancel();
        }
        token
    }

    /// Cancels the grace period because the plugin got a new instance.
    pub fn disarm(&self, plugin_id: &PluginId) {
        if let Some(token) = self.lock().remove(plugin_id) {
            token.cancel();
        }
    }

    /// The map only holds tokens, so a poisoned lock cannot leave it inconsistent.
    fn lock(&self) -> MutexGuard<'_, HashMap<PluginId, CancellationToken>> {
        self.armed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Waits for `grace` unless `token` is cancelled first.
pub async fn wait_for_idle(token: CancellationToken, grace: Duration) -> IdleOutcome {
    tokio::select! {
        _ = token.cancelled() => IdleOutcome::Cancelled,
        _ = tokio::time::sleep(grace) => IdleOutcome::Expired,
    }
}

#[cfg(test)]
mod tests {
    use super::{IDLE_GRACE, IdleOutcome, IdleTimers, wait_for_idle};
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    /// Verifies a reopen (disarm) before the grace period cancels the pending stop, a fresh
    /// arm afterwards expires on schedule, and re-arming cancels the older timer.
    #[tokio::test(start_paused = true)]
    async fn reopen_cancels_pending_idle_stop() {
        let timers = IdleTimers::default();
        let plugin = PluginId::new("official", "acme.hub").expect("plugin id");

        let first = tokio::spawn(wait_for_idle(timers.arm(&plugin), IDLE_GRACE));
        tokio::time::advance(Duration::from_secs(10)).await;
        timers.disarm(&plugin);
        let first = first.await.expect("first wait");

        let second = tokio::spawn(wait_for_idle(timers.arm(&plugin), IDLE_GRACE));
        let third = tokio::spawn(wait_for_idle(timers.arm(&plugin), IDLE_GRACE));
        tokio::time::advance(IDLE_GRACE + Duration::from_millis(1)).await;
        let (second, third) = (
            second.await.expect("second wait"),
            third.await.expect("third wait"),
        );

        assert_eq!(
            (first, second, third),
            (
                IdleOutcome::Cancelled,
                IdleOutcome::Cancelled,
                IdleOutcome::Expired
            )
        );
    }
}

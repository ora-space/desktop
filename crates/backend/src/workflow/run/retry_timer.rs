//! The backend timer behind the engine's automatic-retry wake-ups.

use super::engine::ConcreteWorkflowRunEngine;
use crate::clock::SystemClock;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{Clock, WorkflowRetryTimer};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId};
use ora_logging::{ora_error, ora_warn};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

/// Wakes waiting attempts with one tokio sleep per armed deadline.
///
/// Nothing is kept per wait: the persisted `payload.retry_wait` is the only state, and the
/// engine's `wake_retry` re-checks it under the run lock, so a wake after cancel, run failure,
/// restart, or an earlier wake is a no-op and several waits simply own separate sleeps.
pub(crate) struct WorkflowRetryTimers {
    /// Weak so the engine that owns this timer is not kept alive by it.
    engine: RwLock<Option<Weak<ConcreteWorkflowRunEngine>>>,
    run_locks: Arc<KeyedResourceLocks>,
    clock: SystemClock,
}

impl WorkflowRetryTimers {
    /// Creates a timer with no engine attached yet; the engine embeds the timer, so the
    /// composition root attaches it after building the engine.
    pub(crate) fn new(run_locks: Arc<KeyedResourceLocks>, clock: SystemClock) -> Self {
        Self {
            engine: RwLock::new(None),
            run_locks,
            clock,
        }
    }

    /// Attaches the engine whose `wake_retry` the timer calls.
    pub(crate) fn set_engine(&self, engine: &Arc<ConcreteWorkflowRunEngine>) {
        if let Ok(mut guard) = self.engine.write() {
            *guard = Some(Arc::downgrade(engine));
        }
    }
}

impl WorkflowRetryTimer for WorkflowRetryTimers {
    fn arm(&self, run_id: &WorkflowRunId, node_run_id: &WorkflowNodeRunId, due_at: i64) {
        let Some(engine) = self.engine.read().ok().and_then(|guard| guard.clone()) else {
            ora_warn!(run_id = %run_id, node_run_id = %node_run_id, "retry timer armed before the engine was attached");
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, "retry timer armed outside the tokio runtime");
            return;
        };
        let delay = Duration::from_millis(
            u64::try_from(due_at.saturating_sub(self.clock.now_timestamp_millis())).unwrap_or(0),
        );
        let run_locks = self.run_locks.clone();
        let run_id = run_id.clone();
        let node_run_id = node_run_id.clone();
        runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            // The wake enters the per-run blocking lock and rusqlite, so it runs on the blocking
            // pool like every other engine callback.
            let wake = tokio::task::spawn_blocking(move || {
                let Some(engine) = engine.upgrade() else {
                    return;
                };
                let _gate = run_locks.acquire_exclusive(run_id.as_ref());
                if let Err(error) = engine.wake_retry(&run_id, &node_run_id) {
                    ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %error, "workflow retry wake failed");
                }
            });
            if let Err(source) = wake.await {
                ora_warn!("workflow retry wake panicked: {source}");
            }
        });
    }
}

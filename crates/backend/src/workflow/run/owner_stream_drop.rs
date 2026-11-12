//! E2E-only control for abandoning a workflow executor's owning prompt stream.
//!
//! The production executor owns the prompt stream for an automatic first turn, so a test cannot
//! drop it directly. A test arms one node, waits until the executor claims its real session,
//! confirms that the fake provider received the prompt, and then asks the executor to abandon
//! that same stream through `SessionEventStream::cancel_and_wait`. The subsequent `recv()`
//! returns `None`, which is the production missing-`Completed` path; no stop cause or provider
//! event is fabricated. This module is compiled only with the `test-support` feature.

use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// One Backend's instance-local, one-shot owner-stream drop control.
#[derive(Default)]
pub(crate) struct OwnerStreamDropControl {
    target: Mutex<Option<Arc<Target>>>,
}

/// The armed node and the session the executor claimed for it.
struct Target {
    node_id: String,
    session_id: Mutex<Option<String>>,
    claimed: Notify,
    drop_requested: Notify,
}

/// The executor's handle to one armed target.
pub(crate) struct OwnerStreamDropWaiter {
    target: Arc<Target>,
}

impl OwnerStreamDropControl {
    /// Arms the one node whose owning prompt stream the test will abandon.
    pub(crate) fn arm(&self, node_id: &str) {
        *self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(Target {
            node_id: node_id.to_string(),
            session_id: Mutex::new(None),
            claimed: Notify::new(),
            drop_requested: Notify::new(),
        }));
    }

    /// Claims the armed node once the executor has started consuming its real prompt stream.
    pub(crate) fn claim(&self, node_id: &str, session_id: &str) -> Option<OwnerStreamDropWaiter> {
        let target = self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned()?;
        if target.node_id != node_id {
            return None;
        }
        {
            let mut claimed_session = target
                .session_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if claimed_session.is_some() {
                return None;
            }
            *claimed_session = Some(session_id.to_string());
        }
        target.claimed.notify_one();
        Some(OwnerStreamDropWaiter { target })
    }

    /// Waits for the executor to claim the armed node and returns its real session id.
    pub(crate) async fn wait_for_claimed_session(&self) -> Option<String> {
        let target = self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned()?;
        target.claimed.notified().await;
        target
            .session_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Confirms that the provider received this session's prompt and requests the drop.
    pub(crate) fn confirm_prompt_received(&self, session_id: &str) -> Result<(), String> {
        let target = self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| "owner stream drop was not armed".to_string())?;
        let claimed = target
            .session_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if claimed.as_deref() != Some(session_id) {
            return Err(format!(
                "owner stream drop expected session {claimed:?}, got {session_id:?}"
            ));
        }
        target.drop_requested.notify_one();
        Ok(())
    }
}

impl OwnerStreamDropWaiter {
    /// Resolves once the test has confirmed prompt receipt and requested the owner drop.
    pub(crate) async fn wait_for_drop(&self) {
        self.target.drop_requested.notified().await;
    }
}

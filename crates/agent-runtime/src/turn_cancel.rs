//! A caller-owned cancel of the current prompt with its own answer deadline.
//!
//! The desktop stop button cancels with the runtime's short grace and keeps whatever the provider
//! did. A Node `CancelTurn` instead names an absolute deadline (its acceptance time plus the cancel
//! grace) and must learn whether the Agent answered by then. That request is shared state rather
//! than only a command because three places must see it without waiting for the actor's command
//! loop: the gap between admission and the prompt write, the watchdog's cancel wait, and its
//! resend. The first request of a prompt fixes the deadline; later ones never move it.
use std::sync::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

/// The pending turn cancel of one session actor.
#[derive(Debug, Default)]
pub(crate) struct TurnCancelState {
    deadline: Mutex<Option<Instant>>,
    changed: Notify,
    /// Turn traffic the idle actor settled since the last prompt was admitted. An idle actor
    /// answers such traffic at once, so it is remembered rather than left in the queue.
    idle_traffic: std::sync::atomic::AtomicBool,
}

impl TurnCancelState {
    /// Records a cancel unless one is already pending; the first deadline wins.
    pub(crate) fn request(&self, deadline: Instant) {
        let mut slot = self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(deadline);
            self.changed.notify_waiters();
        }
    }

    /// The pending cancel's deadline, if one was requested for the current prompt.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Forgets a request that belonged to an earlier prompt; called when a new prompt arrives,
    /// before its admission commits, so a late cancel of the previous turn cannot stop this one.
    pub(crate) fn clear(&self) {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.idle_traffic
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Remembers that the idle actor settled turn traffic.
    pub(crate) fn note_idle_traffic(&self) {
        self.idle_traffic
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether turn traffic arrived since the last prompt; reading it forgets it.
    pub(crate) fn take_idle_traffic(&self) -> bool {
        self.idle_traffic
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Resolves once a cancel is requested; callers re-read [`Self::deadline`] afterwards.
    pub(crate) async fn requested(&self) {
        loop {
            let changed = self.changed.notified();
            if self.deadline().is_some() {
                return;
            }
            changed.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TurnCancelState;
    use pretty_assertions::assert_eq;
    use std::time::Duration;
    use tokio::time::Instant;

    /// A second request never moves the deadline the first one fixed.
    #[tokio::test(start_paused = true)]
    async fn the_first_deadline_is_kept() {
        let state = TurnCancelState::default();
        let first = Instant::now() + Duration::from_secs(15);
        state.request(first);
        state.request(first + Duration::from_secs(10));
        assert_eq!(state.deadline(), Some(first));
        state.requested().await;
        state.clear();
        assert_eq!(state.deadline(), None);
    }
}

use std::time::Instant;

use ora_process_protocol::StopRequest;

use crate::{StopError, StopSignal};

/// A stop deadline and successfully delivered actions belong to the accepted responsibility.
pub(crate) struct StopPlan {
    deadline: Instant,
    notify: bool,
    notified: bool,
    forced: bool,
}

impl StopPlan {
    /// Rejects an unrepresentable timeout instead of silently extending responsibility forever.
    pub(crate) fn new(request: StopRequest, now: Instant) -> Result<Self, StopError> {
        let (deadline, notify) = match request {
            StopRequest::Force => (now, false),
            StopRequest::Wait { timeout } => (
                now.checked_add(timeout)
                    .ok_or(StopError::DeadlineOverflow)?,
                false,
            ),
            StopRequest::NotifyThenWait { timeout } => (
                now.checked_add(timeout)
                    .ok_or(StopError::DeadlineOverflow)?,
                true,
            ),
        };
        Ok(Self {
            deadline,
            notify,
            notified: false,
            forced: false,
        })
    }

    /// Retransmission cannot revoke an accepted action or postpone its deadline.
    pub(crate) fn tighten(&mut self, incoming: &Self) {
        self.deadline = self.deadline.min(incoming.deadline);
        self.notify |= incoming.notify;
    }

    /// Force supersedes notification once the earliest accepted deadline has elapsed.
    pub(crate) fn pending(&self, now: Instant) -> Option<StopSignal> {
        if self.forced {
            None
        } else if now >= self.deadline {
            Some(StopSignal::Force)
        } else if self.notify && !self.notified {
            Some(StopSignal::RequestExit)
        } else {
            None
        }
    }

    /// Failed deliveries are not acknowledged and remain retryable on the next reconciliation.
    pub(crate) fn delivered(&mut self, signal: StopSignal) {
        match signal {
            StopSignal::RequestExit => self.notified = true,
            StopSignal::Force => self.forced = true,
        }
    }
}

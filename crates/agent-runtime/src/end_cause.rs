//! Attribution of why one prompt turn ended.
//!
//! Every path that ends a turn early records the same `Cancelled` stop reason in history: the user
//! cancelling from the session UI, the inactivity policy giving up, the client abandoning the
//! stream, and the session dying are indistinguishable there. Only the actor sees which request or
//! control ended the turn, so it reports the cause once and the turn's owner reads it after its
//! stream settled (workflow park) instead of inferring it from the stop reason.

use std::sync::{Arc, Mutex};

/// Why one prompt turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEndCause {
    /// The provider answered the prompt; its stop reason describes the turn.
    Settled,
    /// The user cancelled the prompt from the session UI.
    UserStopped,
    /// The prompt-inactivity policy exhausted its retry windows.
    InactivityTimeout,
    /// The owner abandoned the stream before the turn settled.
    StreamDropped,
    /// The agent session ended without answering.
    SessionCrashed,
    /// The end could not be attributed to a known side.
    Unknown,
}

/// Write-once report of how one prompt turn ended.
///
/// The first writer wins: a turn settles at most once, and a cancellation request that arrives
/// after it settled must not rewrite the reason its owner already saw.
#[derive(Debug, Clone, Default)]
pub struct PromptEndReport {
    cause: Arc<Mutex<Option<PromptEndCause>>>,
}

impl PromptEndReport {
    /// Builds an empty report for one prompt operation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the cause unless this turn already reported one.
    pub fn record(&self, cause: PromptEndCause) {
        let mut slot = self
            .cause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(cause);
        }
    }

    /// Returns the recorded cause once the turn has ended.
    pub fn recorded(&self) -> Option<PromptEndCause> {
        *self
            .cause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{PromptEndCause, PromptEndReport};
    use pretty_assertions::assert_eq;

    /// The first recorded cause survives later requests, so a late cancellation cannot rewrite it.
    #[test]
    fn the_first_recorded_cause_wins() {
        let report = PromptEndReport::new();
        assert_eq!(report.recorded(), None);
        report.record(PromptEndCause::UserStopped);
        report.record(PromptEndCause::StreamDropped);
        assert_eq!(report.recorded(), Some(PromptEndCause::UserStopped));
    }

    /// Clones share one slot, so the actor and the stream owner read the same cause.
    #[test]
    fn clones_share_the_recorded_cause() {
        let report = PromptEndReport::new();
        let reader = report.clone();
        report.record(PromptEndCause::InactivityTimeout);
        assert_eq!(reader.recorded(), Some(PromptEndCause::InactivityTimeout));
    }
}

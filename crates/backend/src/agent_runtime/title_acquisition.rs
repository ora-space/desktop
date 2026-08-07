use ora_scheduler::DelayHandle;

/// Identifies which bounded fallback attempt woke the session actor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PollAttempt {
    First,
    Final,
}

/// Owns the non-persisted first-title acquisition window for one newly attached actor.
pub(super) enum TitleAcquisition {
    Disabled,
    AwaitingFirstEligiblePrompt {
        list_supported: bool,
    },
    Polling {
        list_supported: bool,
        first_handle: Option<DelayHandle>,
        final_handle: Option<DelayHandle>,
    },
    Locked,
}

impl TitleAcquisition {
    /// Creates the open state used only by a newly attached Ora session.
    pub(super) fn awaiting_first_prompt(list_supported: bool) -> Self {
        Self::AwaitingFirstEligiblePrompt { list_supported }
    }

    /// Creates the closed state used when restoring an existing session actor.
    pub(super) fn disabled() -> Self {
        Self::Disabled
    }

    /// Creates the closed state used after an agent switch or a completed acquisition window.
    pub(super) fn locked() -> Self {
        Self::Locked
    }

    /// Reports whether an ACP title update is still allowed to change persistence.
    pub(super) fn accepts_title(&self) -> bool {
        matches!(
            self,
            Self::AwaitingFirstEligiblePrompt { .. } | Self::Polling { .. }
        )
    }

    /// Returns whether the first eligible prompt still needs to open the polling window.
    pub(super) fn list_supported_before_prompt(&self) -> Option<bool> {
        match self {
            Self::AwaitingFirstEligiblePrompt { list_supported } => Some(*list_supported),
            Self::Disabled | Self::Polling { .. } | Self::Locked => None,
        }
    }

    /// Stores scheduler handles so dropping or closing the actor cancels both pending attempts.
    pub(super) fn start_polling(
        &mut self,
        list_supported: bool,
        first_handle: Option<DelayHandle>,
        final_handle: DelayHandle,
    ) {
        if matches!(self, Self::AwaitingFirstEligiblePrompt { .. }) {
            *self = Self::Polling {
                list_supported,
                first_handle,
                final_handle: Some(final_handle),
            };
        }
    }

    /// Releases the handle for a fired attempt while preserving the other scheduled attempt.
    pub(super) fn finish_attempt(&mut self, attempt: PollAttempt) {
        if let Self::Polling {
            first_handle,
            final_handle,
            ..
        } = self
        {
            match attempt {
                PollAttempt::First => drop(first_handle.take()),
                PollAttempt::Final => drop(final_handle.take()),
            }
        }
    }

    /// Preempts one active list attempt while preserving the later fallback when possible.
    pub(super) fn preempt_attempt(&mut self, attempt: PollAttempt) {
        self.finish_attempt(attempt);
        if matches!(attempt, PollAttempt::Final) {
            self.close();
        }
    }

    /// Reports whether this attempt has an ACP list request to perform.
    pub(super) fn should_send_list(&self, attempt: PollAttempt) -> bool {
        match self {
            Self::Polling {
                list_supported,
                first_handle,
                final_handle,
            } => {
                *list_supported
                    && match attempt {
                        PollAttempt::First => first_handle.is_some(),
                        PollAttempt::Final => final_handle.is_some(),
                    }
            }
            Self::Disabled | Self::AwaitingFirstEligiblePrompt { .. } | Self::Locked => false,
        }
    }

    /// Closes the acquisition window and drops every not-yet-fired scheduler handle.
    pub(super) fn close(&mut self) {
        *self = Self::Locked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::UTC;
    use ora_scheduler::Scheduler;
    use std::time::Duration;

    /// Keeps the final list attempt alive after the first attempt has fired.
    #[tokio::test]
    async fn list_attempts_have_independent_lifetimes() {
        let scheduler = Scheduler::new(UTC);
        let first = scheduler
            .schedule_after(Duration::from_secs(60), async {})
            .expect("scheduler accepts first test handle");
        let final_handle = scheduler
            .schedule_after(Duration::from_secs(60), async {})
            .expect("scheduler accepts final test handle");
        let mut acquisition = TitleAcquisition::awaiting_first_prompt(true);
        acquisition.start_polling(true, Some(first), final_handle);

        assert!(acquisition.should_send_list(PollAttempt::First));
        assert!(acquisition.should_send_list(PollAttempt::Final));

        acquisition.finish_attempt(PollAttempt::First);
        assert!(!acquisition.should_send_list(PollAttempt::First));
        assert!(acquisition.should_send_list(PollAttempt::Final));

        acquisition.finish_attempt(PollAttempt::Final);
        assert!(!acquisition.should_send_list(PollAttempt::Final));
        scheduler.shutdown().await;
    }

    /// Keeps an agent without session/list support in the push-only waiting window.
    #[tokio::test]
    async fn unsupported_list_does_not_poll_at_either_boundary() {
        let scheduler = Scheduler::new(UTC);
        let final_handle = scheduler
            .schedule_after(Duration::from_secs(60), async {})
            .expect("scheduler accepts final test handle");
        let mut acquisition = TitleAcquisition::awaiting_first_prompt(false);
        acquisition.start_polling(false, None, final_handle);

        assert!(!acquisition.should_send_list(PollAttempt::First));
        assert!(!acquisition.should_send_list(PollAttempt::Final));
        scheduler.shutdown().await;
    }

    /// Keeps the final fallback alive when the first list request is preempted.
    #[tokio::test]
    async fn preempting_first_attempt_preserves_final_attempt() {
        let scheduler = Scheduler::new(UTC);
        let first = scheduler
            .schedule_after(Duration::from_secs(60), async {})
            .expect("scheduler accepts first test handle");
        let final_handle = scheduler
            .schedule_after(Duration::from_secs(60), async {})
            .expect("scheduler accepts final test handle");
        let mut acquisition = TitleAcquisition::awaiting_first_prompt(true);
        acquisition.start_polling(true, Some(first), final_handle);

        acquisition.preempt_attempt(PollAttempt::First);

        assert!(!acquisition.should_send_list(PollAttempt::First));
        assert!(acquisition.should_send_list(PollAttempt::Final));
        scheduler.shutdown().await;
    }

    /// Locks the acquisition window when the final fallback is preempted.
    #[tokio::test]
    async fn preempting_final_attempt_locks_title() {
        let scheduler = Scheduler::new(UTC);
        let final_handle = scheduler
            .schedule_after(Duration::from_secs(60), async {})
            .expect("scheduler accepts final test handle");
        let mut acquisition = TitleAcquisition::awaiting_first_prompt(true);
        acquisition.start_polling(true, None, final_handle);

        acquisition.preempt_attempt(PollAttempt::Final);

        assert!(!acquisition.accepts_title());
        assert!(!acquisition.should_send_list(PollAttempt::Final));
        scheduler.shutdown().await;
    }
}

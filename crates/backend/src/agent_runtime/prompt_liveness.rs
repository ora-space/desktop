use agent_client_protocol_schema::v1::{SessionUpdate, ToolCallId, ToolCallStatus};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::Instant;

/// Session-local inactivity policy for one active prompt.
///
/// The prompt is given a schedule of meaningful-activity windows: the first covers the
/// initial send and each later one covers an automatic retry of the same prompt after
/// the previous window expired without progress. Within a window every meaningful
/// update rearms the full window; only expiry moves to the next one.
pub(super) struct PromptLiveness {
    windows: &'static [Duration],
    /// Index of the window currently in force; `windows.len() - 1` is the last attempt.
    attempt: usize,
    deadline: Instant,
    running_tools: HashSet<ToolCallId>,
    pending_tools: HashSet<ToolCallId>,
}

/// Identifies one automatic re-send of a stalled prompt, counted from the first retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PromptRetry {
    pub(super) retry: u32,
    pub(super) max_retries: u32,
}

impl PromptLiveness {
    /// Starts the first window of `windows` with no tools holding it open.
    ///
    /// `windows` must not be empty: a prompt always has at least its initial window.
    pub(super) fn new(windows: &'static [Duration]) -> Self {
        Self::new_at(windows, Instant::now())
    }

    fn new_at(windows: &'static [Duration], now: Instant) -> Self {
        let mut liveness = Self {
            windows,
            attempt: 0,
            deadline: now,
            running_tools: HashSet::new(),
            pending_tools: HashSet::new(),
        };
        liveness.reset_at(now);
        liveness
    }

    /// Returns a deadline only while ordinary prompt progress is expected.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.running_tools.is_empty().then_some(self.deadline)
    }

    /// Waits until the active deadline, or forever while a running tool pauses it.
    pub(super) async fn wait(&self) {
        match self.deadline() {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    }

    /// Moves to the next window after the current one expired, or reports the schedule
    /// is exhausted so the caller fails the prompt instead of re-sending it.
    ///
    /// The re-sent prompt is a fresh provider turn: tools observed by the stalled
    /// attempt no longer hold or rearm the window, so their bookkeeping is dropped.
    pub(super) fn next_attempt(&mut self) -> Option<PromptRetry> {
        self.next_attempt_at(Instant::now())
    }

    fn next_attempt_at(&mut self, now: Instant) -> Option<PromptRetry> {
        let max_retries = self.windows.len() - 1;
        if self.attempt >= max_retries {
            return None;
        }
        self.attempt += 1;
        self.running_tools.clear();
        self.pending_tools.clear();
        self.reset_at(now);
        Some(PromptRetry {
            retry: u32::try_from(self.attempt).unwrap_or(u32::MAX),
            max_retries: u32::try_from(max_retries).unwrap_or(u32::MAX),
        })
    }

    /// Applies only activity that proves the current prompt, rather than session chrome, moved.
    pub(super) fn observe(&mut self, update: &SessionUpdate) {
        self.observe_at(update, Instant::now());
    }

    fn observe_at(&mut self, update: &SessionUpdate, now: Instant) {
        match update {
            SessionUpdate::AgentMessageChunk(_)
            | SessionUpdate::AgentThoughtChunk(_)
            | SessionUpdate::Plan(_) => self.reset_at(now),
            SessionUpdate::ToolCall(call) => {
                self.observe_tool(&call.tool_call_id, Some(call.status), now);
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.observe_tool(&update.tool_call_id, update.fields.status, now);
            }
            SessionUpdate::UserMessageChunk(_)
            | SessionUpdate::AvailableCommandsUpdate(_)
            | SessionUpdate::CurrentModeUpdate(_)
            | SessionUpdate::ConfigOptionUpdate(_)
            | SessionUpdate::SessionInfoUpdate(_)
            | SessionUpdate::UsageUpdate(_) => {}
            _ => {}
        }
    }

    /// Rearms a full window after an awaited permission round trip.
    pub(super) fn permission_settled(&mut self) {
        self.reset_at(Instant::now());
    }

    fn observe_tool(&mut self, id: &ToolCallId, status: Option<ToolCallStatus>, now: Instant) {
        match status {
            Some(ToolCallStatus::Pending) => {
                if self.pending_tools.insert(id.clone()) {
                    self.reset_at(now);
                }
            }
            Some(ToolCallStatus::InProgress) => {
                self.running_tools.insert(id.clone());
            }
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed) => {
                self.running_tools.remove(id);
                if self.running_tools.is_empty() {
                    self.reset_at(now);
                }
            }
            None | Some(_) => self.reset_at(now),
        }
    }

    fn reset_at(&mut self, now: Instant) {
        self.deadline = now + self.windows[self.attempt];
    }
}

#[cfg(test)]
mod tests {
    use super::{PromptLiveness, PromptRetry};
    use agent_client_protocol_schema::v1::{
        Plan, SessionUpdate, ToolCall, ToolCallStatus, UsageUpdate,
    };
    use pretty_assertions::assert_eq;
    use std::time::Duration;
    use tokio::time::Instant;

    const SINGLE_WINDOW: [Duration; 1] = [Duration::from_secs(60)];
    const WIDENING_WINDOWS: [Duration; 3] = [
        Duration::from_secs(45),
        Duration::from_secs(60),
        Duration::from_secs(90),
    ];

    #[test]
    fn meaningful_activity_rearms_but_session_chrome_does_not() {
        let base = Instant::now();
        let timeout = SINGLE_WINDOW[0];
        let mut liveness = PromptLiveness::new_at(&SINGLE_WINDOW, base);
        let initial = base + timeout;

        liveness.observe_at(
            &SessionUpdate::UsageUpdate(UsageUpdate::new(10, 100)),
            base + Duration::from_secs(30),
        );
        assert_eq!(liveness.deadline(), Some(initial));

        liveness.observe_at(
            &SessionUpdate::Plan(Plan::new(Vec::new())),
            base + Duration::from_secs(60),
        );
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(120)));
    }

    #[test]
    fn pending_rearms_once_and_direct_terminal_rearms() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new_at(&SINGLE_WINDOW, base);
        let pending =
            SessionUpdate::ToolCall(ToolCall::new("tool", "Tool").status(ToolCallStatus::Pending));

        liveness.observe_at(&pending, base + Duration::from_secs(10));
        liveness.observe_at(&pending, base + Duration::from_secs(20));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(70)));

        liveness.observe_at(
            &SessionUpdate::ToolCall(
                ToolCall::new("direct", "Direct").status(ToolCallStatus::Completed),
            ),
            base + Duration::from_secs(30),
        );
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(90)));
    }

    #[test]
    fn parallel_running_tools_pause_only_their_own_prompt_window() {
        let base = Instant::now();
        let timeout = SINGLE_WINDOW[0];
        let mut first = PromptLiveness::new_at(&SINGLE_WINDOW, base);
        let second = PromptLiveness::new_at(&SINGLE_WINDOW, base);
        first.observe_at(
            &SessionUpdate::ToolCall(
                ToolCall::new("same-id", "Long test").status(ToolCallStatus::InProgress),
            ),
            base + Duration::from_secs(10),
        );
        first.observe_at(
            &SessionUpdate::ToolCall(
                ToolCall::new("parallel", "Other test").status(ToolCallStatus::InProgress),
            ),
            base + Duration::from_secs(20),
        );
        assert_eq!(first.deadline(), None);
        assert_eq!(second.deadline(), Some(base + timeout));

        first.observe_at(
            &SessionUpdate::ToolCall(
                ToolCall::new("same-id", "Long test").status(ToolCallStatus::Completed),
            ),
            base + Duration::from_secs(30),
        );
        assert_eq!(first.deadline(), None);
        first.observe_at(
            &SessionUpdate::ToolCall(
                ToolCall::new("parallel", "Other test").status(ToolCallStatus::Failed),
            ),
            base + Duration::from_secs(40),
        );
        assert_eq!(first.deadline(), Some(base + Duration::from_secs(100)));
    }

    #[test]
    fn expiry_walks_the_widening_schedule_then_exhausts() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new_at(&WIDENING_WINDOWS, base);
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(45)));

        let first_expiry = base + Duration::from_secs(45);
        assert_eq!(
            liveness.next_attempt_at(first_expiry),
            Some(PromptRetry {
                retry: 1,
                max_retries: 2,
            }),
        );
        assert_eq!(
            liveness.deadline(),
            Some(first_expiry + Duration::from_secs(60))
        );

        // Progress inside a retry rearms that retry's own window, not the first one.
        let progress = first_expiry + Duration::from_secs(30);
        liveness.observe_at(&SessionUpdate::Plan(Plan::new(Vec::new())), progress);
        assert_eq!(
            liveness.deadline(),
            Some(progress + Duration::from_secs(60))
        );

        let second_expiry = progress + Duration::from_secs(60);
        assert_eq!(
            liveness.next_attempt_at(second_expiry),
            Some(PromptRetry {
                retry: 2,
                max_retries: 2,
            }),
        );
        assert_eq!(
            liveness.deadline(),
            Some(second_expiry + Duration::from_secs(90))
        );

        let third_expiry = second_expiry + Duration::from_secs(90);
        assert_eq!(liveness.next_attempt_at(third_expiry), None);
        assert_eq!(liveness.next_attempt_at(third_expiry), None);
    }

    #[test]
    fn a_retry_drops_the_stalled_attempts_tool_bookkeeping() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new_at(&WIDENING_WINDOWS, base);
        let pending =
            SessionUpdate::ToolCall(ToolCall::new("tool", "Tool").status(ToolCallStatus::Pending));
        liveness.observe_at(&pending, base + Duration::from_secs(10));

        let expiry = base + Duration::from_secs(70);
        liveness.next_attempt_at(expiry);

        // The re-sent prompt may reuse the same tool id; its first pending must rearm again.
        liveness.observe_at(&pending, expiry + Duration::from_secs(20));
        assert_eq!(liveness.deadline(), Some(expiry + Duration::from_secs(80)));
    }

    #[test]
    fn a_single_window_schedule_never_retries() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new_at(&SINGLE_WINDOW, base);

        assert_eq!(
            liveness.next_attempt_at(base + Duration::from_secs(60)),
            None
        );
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(60)));
    }
}

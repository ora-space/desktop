use agent_client_protocol_schema::v1::{SessionUpdate, ToolCallId, ToolCallStatus};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::Instant;

/// Session-local inactivity policy for one active prompt.
pub(super) struct PromptLiveness {
    timeout: Duration,
    deadline: Instant,
    running_tools: HashSet<ToolCallId>,
    pending_tools: HashSet<ToolCallId>,
}

impl PromptLiveness {
    /// Starts a fresh prompt window with no tools holding it open.
    pub(super) fn new(timeout: Duration) -> Self {
        Self::new_at(timeout, Instant::now())
    }

    fn new_at(timeout: Duration, now: Instant) -> Self {
        Self {
            timeout,
            deadline: now + timeout,
            running_tools: HashSet::new(),
            pending_tools: HashSet::new(),
        }
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
        self.deadline = now + self.timeout;
    }
}

#[cfg(test)]
mod tests {
    use super::PromptLiveness;
    use agent_client_protocol_schema::v1::{
        Plan, SessionUpdate, ToolCall, ToolCallStatus, UsageUpdate,
    };
    use pretty_assertions::assert_eq;
    use std::time::Duration;
    use tokio::time::Instant;

    #[test]
    fn meaningful_activity_rearms_but_session_chrome_does_not() {
        let base = Instant::now();
        let timeout = Duration::from_secs(300);
        let mut liveness = PromptLiveness::new_at(timeout, base);
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
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(360)));
    }

    #[test]
    fn pending_rearms_once_and_direct_terminal_rearms() {
        let base = Instant::now();
        let timeout = Duration::from_secs(300);
        let mut liveness = PromptLiveness::new_at(timeout, base);
        let pending =
            SessionUpdate::ToolCall(ToolCall::new("tool", "Tool").status(ToolCallStatus::Pending));

        liveness.observe_at(&pending, base + Duration::from_secs(10));
        liveness.observe_at(&pending, base + Duration::from_secs(20));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(310)));

        liveness.observe_at(
            &SessionUpdate::ToolCall(
                ToolCall::new("direct", "Direct").status(ToolCallStatus::Completed),
            ),
            base + Duration::from_secs(30),
        );
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(330)));
    }

    #[test]
    fn parallel_running_tools_pause_only_their_own_prompt_window() {
        let base = Instant::now();
        let timeout = Duration::from_secs(300);
        let mut first = PromptLiveness::new_at(timeout, base);
        let second = PromptLiveness::new_at(timeout, base);
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
        assert_eq!(first.deadline(), Some(base + Duration::from_secs(340)));
    }
}

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
        Self {
            timeout,
            deadline: Instant::now() + timeout,
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
        match update {
            SessionUpdate::AgentMessageChunk(_)
            | SessionUpdate::AgentThoughtChunk(_)
            | SessionUpdate::Plan(_) => self.reset(),
            SessionUpdate::ToolCall(call) => self.observe_tool(&call.tool_call_id, Some(call.status)),
            SessionUpdate::ToolCallUpdate(update) => {
                self.observe_tool(&update.tool_call_id, update.fields.status);
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
        self.reset();
    }

    fn observe_tool(&mut self, id: &ToolCallId, status: Option<ToolCallStatus>) {
        match status {
            Some(ToolCallStatus::Pending) => {
                if self.pending_tools.insert(id.clone()) {
                    self.reset();
                }
            }
            Some(ToolCallStatus::InProgress) => {
                self.running_tools.insert(id.clone());
            }
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed) => {
                if self.running_tools.remove(id) && self.running_tools.is_empty() {
                    self.reset();
                }
            }
            None | Some(_) => self.reset(),
        }
    }

    fn reset(&mut self) {
        self.deadline = Instant::now() + self.timeout;
    }
}

#[cfg(test)]
mod tests {
    use super::PromptLiveness;
    use agent_client_protocol_schema::v1::{SessionUpdate, ToolCall, ToolCallStatus};
    use std::time::Duration;

    #[test]
    fn running_tools_pause_only_their_own_prompt_window() {
        let mut first = PromptLiveness::new(Duration::from_secs(300));
        let second = PromptLiveness::new(Duration::from_secs(300));
        first.observe(&SessionUpdate::ToolCall(
            ToolCall::new("same-id", "Long test").status(ToolCallStatus::InProgress),
        ));
        assert_eq!(first.deadline(), None);
        assert!(second.deadline().is_some());
        assert_ne!(first.deadline(), second.deadline());
    }
}

use agent_client_protocol_schema::v1::{
    Meta, SessionUpdate, ToolCallId, ToolCallStatus, ToolCallUpdate,
};
use ora_domain::PromptInactivityPolicy;
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::Instant;

/// Which side of a parent/child projection one update's metadata names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Projection {
    /// Metadata carries a valid child source, so the update only displays child work.
    Projected,
    /// Metadata is omitted, so ownership already recorded for this tool still applies.
    Inherited,
    /// Metadata clears or invalidates the source, returning the tool to its parent.
    Parent,
}

impl Projection {
    /// Classifies metadata with the history crate's source rule, so the timer and the transcript
    /// agree on which sources count as a child (ADR D1).
    fn of(meta: Option<&Meta>) -> Self {
        if !ora_history::has_child_source_namespace(meta) {
            return Self::Inherited;
        }
        // The namespace is opaque replacement data: an explicit null or a malformed object
        // clears every earlier source instead of naming a child (ADR D2).
        if ora_history::child_source_id(meta).is_some() {
            Self::Projected
        } else {
            Self::Parent
        }
    }
}

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
    seen_pending_tools: HashSet<ToolCallId>,
    /// Projection ownership survives later deltas that omit metadata for the same tool.
    projected_tools: HashSet<ToolCallId>,
}

/// Identifies one automatic re-send of a stalled prompt, counted from the first retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PromptRetry {
    pub(super) retry: u32,
    pub(super) max_retries: u32,
}

/// Bounded, content-free evidence explaining an inactivity decision for the current attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PromptLivenessSnapshot {
    pub(super) attempt: usize,
    pub(super) window: Duration,
    pub(super) silent_elapsed: Duration,
    pub(super) running_tools: usize,
    pub(super) pending_tools: usize,
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
            seen_pending_tools: HashSet::new(),
            projected_tools: HashSet::new(),
        };
        liveness.reset_at(now);
        liveness
    }

    /// Returns a deadline only while ordinary prompt progress is expected.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.running_tools.is_empty().then_some(self.deadline)
    }

    /// Captures the exact window and tool ownership used by an inactivity decision.
    pub(super) fn snapshot(&self) -> PromptLivenessSnapshot {
        let window = self.windows[self.attempt];
        PromptLivenessSnapshot {
            attempt: self.attempt + 1,
            window,
            silent_elapsed: Instant::now().saturating_duration_since(self.deadline - window),
            running_tools: self.running_tools.len(),
            pending_tools: self.pending_tools.len(),
        }
    }

    /// Waits only for an enabled deadline; tools and an explicit wait policy keep other actor
    /// inputs responsive without treating silence as permission to cancel the prompt.
    pub(super) async fn wait(&self, policy: PromptInactivityPolicy) {
        match policy {
            PromptInactivityPolicy::Timeout => match self.deadline() {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            },
            PromptInactivityPolicy::Wait => std::future::pending().await,
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
        self.seen_pending_tools.clear();
        self.projected_tools.clear();
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
        // A display projection is not proof that this prompt awaits that child. Parent delegate
        // tools keep their existing in-progress pause; child output cannot create another pause.
        let (meta, tool) = match update {
            SessionUpdate::AgentMessageChunk(chunk) | SessionUpdate::AgentThoughtChunk(chunk) => {
                (&chunk.meta, None)
            }
            SessionUpdate::Plan(plan) => (&plan.meta, None),
            SessionUpdate::ToolCall(call) => (&call.meta, Some(&call.tool_call_id)),
            SessionUpdate::ToolCallUpdate(change) => (&change.meta, Some(&change.tool_call_id)),
            _ => (&None, None),
        };
        let projection = Projection::of(meta.as_ref());
        // D6: a CodeAgent subagent frame proves its open parent delegation moved, so it rearms
        // the parent window without pausing it. Only a delegation still pending or in progress
        // in this attempt qualifies; completed, failed, unknown or absent outer ids do not
        // renew, and the delegate's own in-progress pause keeps priority over renewal.
        if projection == Projection::Projected
            && let Some(delegate) = ora_history::subagent_delegation_tool_call_id(meta.as_ref())
        {
            let delegate = ToolCallId::new(delegate);
            if self.pending_tools.contains(&delegate) || self.running_tools.contains(&delegate) {
                self.reset_at(now);
            }
        }
        if let Some(id) = tool {
            match projection {
                Projection::Projected => {
                    self.projected_tools.insert(id.clone());
                    self.running_tools.remove(id);
                    self.pending_tools.remove(id);
                    return;
                }
                // A cleared or malformed source republishes the same tool as the parent's, so
                // stop discarding its later deltas before this one is applied.
                Projection::Parent => {
                    self.projected_tools.remove(id);
                }
                Projection::Inherited => {
                    if self.projected_tools.contains(id) {
                        return;
                    }
                }
            }
        } else if projection == Projection::Projected {
            return;
        }
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
    pub(super) fn permission_settled(&mut self, tool: &ToolCallUpdate) {
        match Projection::of(tool.meta.as_ref()) {
            // A child's round trip is not parent progress.
            Projection::Projected => {}
            // Explicitly cleared sources belong to the parent again, whose awaited round trips
            // always rearm.
            Projection::Parent => {
                self.projected_tools.remove(&tool.tool_call_id);
                self.reset_at(Instant::now());
            }
            Projection::Inherited => {
                if !self.projected_tools.contains(&tool.tool_call_id) {
                    self.reset_at(Instant::now());
                }
            }
        }
    }

    fn observe_tool(&mut self, id: &ToolCallId, status: Option<ToolCallStatus>, now: Instant) {
        match status {
            Some(ToolCallStatus::Pending) => {
                self.pending_tools.insert(id.clone());
                if self.seen_pending_tools.insert(id.clone()) {
                    self.reset_at(now);
                }
            }
            Some(ToolCallStatus::InProgress) => {
                self.pending_tools.remove(id);
                self.running_tools.insert(id.clone());
            }
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed) => {
                self.pending_tools.remove(id);
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
    use super::{PromptLiveness, PromptLivenessSnapshot, PromptRetry};
    use agent_client_protocol_schema::v1::{
        Meta, Plan, SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
        UsageUpdate,
    };
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::time::Duration;
    use tokio::time::Instant;

    const SINGLE_WINDOW: [Duration; 1] = [Duration::from_secs(60)];
    const WIDENING_WINDOWS: [Duration; 3] = [
        Duration::from_secs(45),
        Duration::from_secs(60),
        Duration::from_secs(90),
    ];

    /// Builds the metadata one update carries for the `opencode/child-session` namespace.
    fn child_meta(source: &Value) -> Meta {
        serde_json::from_value(json!({ "opencode/child-session": source })).unwrap()
    }

    /// Builds an in-progress delta for the delegate tool with the given namespace value.
    fn delegate_in_progress(source: &Value) -> SessionUpdate {
        SessionUpdate::ToolCallUpdate(
            ToolCallUpdate::new(
                "delegate",
                ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
            )
            .meta(child_meta(source)),
        )
    }

    /// A source the history crate accepts, so both consumers agree it names a child.
    fn valid_source() -> Value {
        json!({"id":"child","parentID":"root","depth":1})
    }

    /// An explicit null republishes a running delegate as the parent's instead of ending its pause.
    #[test]
    fn cleared_child_source_keeps_a_running_delegate_paused() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new_at(&SINGLE_WINDOW, base);
        liveness.observe_at(
            &delegate_in_progress(&Value::Null),
            base + Duration::from_secs(10),
        );
        assert_eq!(liveness.deadline(), None);

        // The tool stays the parent's, so its completion settles the prompt normally.
        liveness.observe_at(
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "delegate",
                ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            )),
            base + Duration::from_secs(20),
        );
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(80)));
    }

    /// Every malformed or cleared source belongs to the parent and cannot end a running pause.
    #[test]
    fn invalid_child_sources_do_not_end_a_running_pause() {
        let base = Instant::now();
        for source in [
            Value::Null,
            json!({}),
            json!({"id":"  ","parentID":"root","depth":1}),
            json!({"id":"child","parentID":"  ","depth":1}),
            json!({"id":"child","parentID":"root","depth":0}),
            json!({"id":"child","parentID":"root","depth":-1}),
            json!({"id":"child","parentID":"root","depth":1.5}),
            json!({"id":"child","parentID":"root","depth":"1"}),
            json!({"id":"child","parentID":"root","depth":9007199254740992u64}),
        ] {
            let mut liveness = PromptLiveness::new_at(&SINGLE_WINDOW, base);
            liveness.observe_at(&delegate_in_progress(&valid_source()), base);
            // A valid source only displays child work: it never pauses the parent prompt.
            assert_eq!(liveness.deadline(), Some(base + SINGLE_WINDOW[0]));
            liveness.observe_at(
                &delegate_in_progress(&source),
                base + Duration::from_secs(10),
            );
            // Clearing it returns the tool to the parent, whose running state pauses the prompt.
            assert_eq!(liveness.deadline(), None);
        }
    }

    /// A cleared source makes the awaited permission round trip the parent's again.
    #[tokio::test(start_paused = true)]
    async fn cleared_child_source_permission_rearms_the_parent_deadline() {
        let mut liveness = PromptLiveness::new(&SINGLE_WINDOW);
        tokio::time::advance(Duration::from_secs(20)).await;
        let cleared = ToolCallUpdate::new("delegate", ToolCallUpdateFields::new())
            .meta(child_meta(&Value::Null));
        liveness.permission_settled(&cleared);
        assert_eq!(liveness.deadline(), Some(Instant::now() + SINGLE_WINDOW[0]));

        // The cleared delta also stops discarding the same tool's later metadata-less deltas.
        liveness.observe(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "delegate",
            ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
        )));
        assert_eq!(liveness.deadline(), None);
    }

    /// A background child's permission roundtrip does not prove parent progress either.
    #[tokio::test(start_paused = true)]
    async fn child_permission_does_not_renew_the_parent_deadline() {
        let mut liveness = PromptLiveness::new(&SINGLE_WINDOW);
        let original = liveness.deadline();
        tokio::time::advance(Duration::from_secs(20)).await;
        let child = serde_json::from_value(serde_json::json!({
            "toolCallId":"child:tool", "_meta":{"opencode/child-session":{"id":"child","parentID":"root","depth":1}}
        })).unwrap();
        liveness.permission_settled(&child);
        assert_eq!(liveness.deadline(), original);
        let parent = serde_json::from_value(serde_json::json!({"toolCallId":"delegate"})).unwrap();
        liveness.permission_settled(&parent);
        assert_eq!(liveness.deadline(), Some(Instant::now() + SINGLE_WINDOW[0]));
    }

    /// Diagnostics distinguish currently pending work from ids remembered for rearm deduplication.
    #[tokio::test(start_paused = true)]
    async fn snapshots_track_current_tools_and_the_attempt_window() {
        let mut liveness = PromptLiveness::new(&WIDENING_WINDOWS);
        let pending = SessionUpdate::ToolCall(
            ToolCall::new("delegate", "Explore").status(ToolCallStatus::Pending),
        );
        liveness.observe(&pending);
        tokio::time::advance(Duration::from_secs(10)).await;
        liveness.observe(&pending);
        assert_eq!(
            liveness.snapshot(),
            PromptLivenessSnapshot {
                attempt: 1,
                window: Duration::from_secs(45),
                silent_elapsed: Duration::from_secs(10),
                running_tools: 0,
                pending_tools: 1,
            }
        );
        liveness.observe(&SessionUpdate::ToolCall(
            ToolCall::new("delegate", "Explore").status(ToolCallStatus::InProgress),
        ));
        assert_eq!(
            liveness.snapshot(),
            PromptLivenessSnapshot {
                attempt: 1,
                window: Duration::from_secs(45),
                silent_elapsed: Duration::from_secs(10),
                running_tools: 1,
                pending_tools: 0,
            }
        );
        liveness.observe(&SessionUpdate::ToolCall(
            ToolCall::new("delegate", "Explore").status(ToolCallStatus::Completed),
        ));
        liveness.observe(&SessionUpdate::ToolCall(
            ToolCall::new("rejected", "Rejected").status(ToolCallStatus::Pending),
        ));
        liveness.observe(&SessionUpdate::ToolCall(
            ToolCall::new("rejected", "Rejected").status(ToolCallStatus::Failed),
        ));
        assert_eq!(
            liveness.snapshot(),
            PromptLivenessSnapshot {
                attempt: 1,
                window: Duration::from_secs(45),
                silent_elapsed: Duration::ZERO,
                running_tools: 0,
                pending_tools: 0,
            }
        );
        liveness.next_attempt();
        liveness.observe(&pending);
        assert_eq!(
            liveness.snapshot(),
            PromptLivenessSnapshot {
                attempt: 2,
                window: Duration::from_secs(60),
                silent_elapsed: Duration::ZERO,
                running_tools: 0,
                pending_tools: 1,
            }
        );
    }

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

    const DEFAULT_WINDOW: [Duration; 1] = [Duration::from_secs(45)];

    /// Builds the parent `Task` opening exactly as the captured s1 sequence sends it.
    fn parent_delegate(status: ToolCallStatus) -> SessionUpdate {
        SessionUpdate::ToolCall(ToolCall::new("call-1", "Task").status(status))
    }

    /// Builds one normalized `_subagent/update` chunk as the connection translator emits it.
    fn subagent_chunk(delegate: Option<&str>) -> SessionUpdate {
        let mut source = json!({"sessionId":"sess-1","taskId":"task-1"});
        if let Some(delegate) = delegate {
            source["toolCallId"] = json!(delegate);
        }
        serde_json::from_value(json!({
            "sessionUpdate":"agent_message_chunk",
            "content":{"type":"text","text":"child progress"},
            "_meta":{"_subagent/update":source},
        }))
        .unwrap()
    }

    /// Builds one normalized subagent tool call whose inner id is already task-scoped.
    fn subagent_tool_call(delegate: &str) -> SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate":"tool_call",
            "toolCallId":"subagent:6:task-1:call-2",
            "title":"Bash",
            "status":"pending",
            "_meta":{"_subagent/update":{"sessionId":"sess-1","taskId":"task-1","toolCallId":delegate}},
        }))
        .unwrap()
    }

    /// D6: child frames rearms the parent window while the delegation is open, and stop once the
    /// provider completed it; renewal never pauses and never counts the child as a running tool.
    #[tokio::test(start_paused = true)]
    async fn subagent_frames_renew_the_parent_window_only_while_the_delegation_is_open() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new(&DEFAULT_WINDOW);
        liveness.observe(&parent_delegate(ToolCallStatus::Pending));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(45)));

        tokio::time::advance(Duration::from_secs(10)).await;
        liveness.observe(&subagent_chunk(Some("call-1")));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(55)));
        // A child tool frame renews the same way and stays out of the parent's tool bookkeeping.
        tokio::time::advance(Duration::from_secs(5)).await;
        liveness.observe(&subagent_tool_call("call-1"));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(60)));
        assert_eq!(
            liveness.snapshot(),
            PromptLivenessSnapshot {
                attempt: 1,
                window: Duration::from_secs(45),
                silent_elapsed: Duration::ZERO,
                running_tools: 0,
                pending_tools: 1,
            }
        );

        // The provider completes the delegation; later child frames must not keep renewing.
        tokio::time::advance(Duration::from_secs(5)).await;
        liveness.observe(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "call-1",
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        )));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(65)));
        tokio::time::advance(Duration::from_secs(10)).await;
        liveness.observe(&subagent_chunk(Some("call-1")));
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(65)));
    }

    /// D6: missing or unknown outer ids, a failed delegation, and other providers' projections
    /// never renew the parent window.
    #[test]
    fn subagent_frames_without_an_open_known_delegation_do_not_renew() {
        let base = Instant::now();
        let later = base + Duration::from_secs(10);
        let unchanged = Some(base + Duration::from_secs(45));

        // Missing outer id: the frame displays child work but names no delegation.
        let mut liveness = PromptLiveness::new_at(&DEFAULT_WINDOW, base);
        liveness.observe_at(&parent_delegate(ToolCallStatus::Pending), base);
        liveness.observe_at(&subagent_chunk(None), later);
        assert_eq!(liveness.deadline(), unchanged);

        // Unknown outer id: no parent tool with that id was observed in this attempt.
        let mut liveness = PromptLiveness::new_at(&DEFAULT_WINDOW, base);
        liveness.observe_at(&parent_delegate(ToolCallStatus::Pending), base);
        liveness.observe_at(&subagent_chunk(Some("call-unknown")), later);
        assert_eq!(liveness.deadline(), unchanged);

        // Failed delegation: the parent call already ended in this attempt.
        let mut liveness = PromptLiveness::new_at(&DEFAULT_WINDOW, base);
        liveness.observe_at(&parent_delegate(ToolCallStatus::Failed), base);
        liveness.observe_at(&subagent_chunk(Some("call-1")), later);
        assert_eq!(liveness.deadline(), unchanged);

        // OpenCode projections carry no outer delegation id and keep the 20261020 exclusion.
        let mut liveness = PromptLiveness::new_at(&DEFAULT_WINDOW, base);
        liveness.observe_at(&parent_delegate(ToolCallStatus::Pending), base);
        liveness.observe_at(
            &SessionUpdate::AgentMessageChunk(
                agent_client_protocol_schema::v1::ContentChunk::new(
                    agent_client_protocol_schema::v1::ContentBlock::Text(
                        agent_client_protocol_schema::v1::TextContent::new("background"),
                    ),
                )
                .meta(child_meta(&valid_source())),
            ),
            later,
        );
        assert_eq!(liveness.deadline(), unchanged);
    }

    /// D6: renewal never lifts the delegate's own in-progress pause, and a retry drops the old
    /// attempt's delegation observations until the re-sent turn observes the delegate again.
    #[test]
    fn subagent_renewal_never_pauses_and_a_retry_clears_the_delegation() {
        let base = Instant::now();
        let mut liveness = PromptLiveness::new_at(&WIDENING_WINDOWS, base);
        liveness.observe_at(&parent_delegate(ToolCallStatus::InProgress), base);
        assert_eq!(liveness.deadline(), None);
        liveness.observe_at(
            &subagent_chunk(Some("call-1")),
            base + Duration::from_secs(10),
        );
        assert_eq!(liveness.deadline(), None);
        liveness.observe_at(
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "call-1",
                ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            )),
            base + Duration::from_secs(20),
        );
        assert_eq!(liveness.deadline(), Some(base + Duration::from_secs(65)));

        // The retry starts a fresh attempt: the stale outer id no longer names an open delegate.
        let expiry = base + Duration::from_secs(65);
        liveness.next_attempt_at(expiry);
        liveness.observe_at(
            &subagent_chunk(Some("call-1")),
            expiry + Duration::from_secs(10),
        );
        assert_eq!(liveness.deadline(), Some(expiry + Duration::from_secs(60)));
        // The re-sent turn observes the delegate again; child frames renew the new window.
        liveness.observe_at(
            &parent_delegate(ToolCallStatus::Pending),
            expiry + Duration::from_secs(20),
        );
        assert_eq!(liveness.deadline(), Some(expiry + Duration::from_secs(80)));
        liveness.observe_at(
            &subagent_chunk(Some("call-1")),
            expiry + Duration::from_secs(30),
        );
        assert_eq!(liveness.deadline(), Some(expiry + Duration::from_secs(90)));
    }
}

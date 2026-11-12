use super::connection::AgentAcpClient;
use super::events::drain_idle_events;
use super::replay::recorded_replay;
use super::routing::{SessionControl, SessionEvent};
use super::tool_timing::ToolTimings;
use super::*;
use crate::AgentRuntimeHost;
#[path = "actor_history.rs"]
mod actor_history;
#[path = "actor_mcp.rs"]
mod actor_mcp;
#[path = "actor_prompt.rs"]
mod actor_prompt;
#[path = "cleanup.rs"]
mod cleanup;
#[path = "title_polling.rs"]
mod title_polling;
mod usage;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::CancelNotification;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{
    CloseSessionRequest, CloseSessionResponse, ConfigOptionUpdate, SessionUpdate,
};
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_logging::{ora_debug, ora_warn};
use tokio::time::timeout;

/// How far replaying Ora's record got before it stopped.
///
/// Only `Delivered` may complete the load stream. The other two are kept apart because
/// they differ in who still has to be told: an unreadable history owes the
/// client an error, while an abandoned one has no client left to send it to.
enum Replay {
    /// Every recorded line reached the client.
    Delivered,
    /// The history could not be read, and the client was told so.
    Unreadable,
    /// The client stopped listening partway through.
    Abandoned,
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Serializes operations for one logical session while the shared connection remains concurrent.
    pub(super) async fn run(mut self) {
        loop {
            let command_sender = self.command_sender.clone();
            let command = match self.channel.as_mut() {
                Some(channel) => {
                    // Residual events belong to the previous provider turn. Consume the current
                    // queue snapshot before accepting a new command so they cannot cross turns.
                    drain_idle_events(
                        &channel.connection.client,
                        &mut channel.events,
                        &command_sender,
                    )
                    .await;
                    if let Ok(control) = channel.controls.try_recv() {
                        self.handle_idle_control(Some(control)).await;
                        continue;
                    }
                    tokio::select! {
                        biased;
                        control = channel.controls.recv() => {
                            self.handle_idle_control(control).await;
                            continue;
                        }
                        command = self.commands.recv() => {
                            drain_idle_events(
                                &channel.connection.client,
                                &mut channel.events,
                                &command_sender,
                            )
                            .await;
                            command
                        }
                        event = channel.events.recv() => {
                            let Some(event) = event else {
                                self.mark_stopped();
                                continue;
                            };
                            if let SessionEvent::Update(update) = &event
                                && let Some(command_sender) = command_sender.upgrade()
                            {
                                let _ = command_sender.send(RuntimeCommand::TitleUpdate {
                                    update: Box::new(update.update.clone()),
                                });
                            }
                            super::events::settle_idle_event(&channel.connection.client, event).await;
                            continue;
                        }
                    }
                }
                None => self.commands.recv().await,
            };
            let Some(command) = command else {
                // The manager dropped this actor, which it only does when it is
                // replacing or deleting the row itself. Detaching from the
                // provider is still required, but persisting anything would write
                // a snapshot the manager has already moved past — that is exactly
                // how a switch's new binding gets reverted to Stopped.
                self.release().await;
                return;
            };
            match command {
                RuntimeCommand::Load {
                    cleanup,
                    operation_id,
                    events,
                    accepted,
                } => {
                    self.run_load(events, accepted).await;
                    self.record_cleanup(operation_id, Ok(()));
                    let _ = cleanup.send(Ok(()));
                }
                RuntimeCommand::Prompt {
                    operation_id,
                    prompt,
                    record_prompt,
                    inactivity_policy,
                    model,
                    end_report,
                    events,
                    accepted,
                } => {
                    // Conversations are opened without a provider, so a send is where one is
                    // acquired. Admission waits on that: a prompt that cannot reach an agent, or
                    // that cannot be given the MCP set configured right now, has to fail as the
                    // send it was, not as a turn that started and produced nothing.
                    match self.ensure_attached(model.as_deref()).await {
                        Ok(setup) => {
                            let _ = accepted.send(Ok(()));
                            if publish_setup(&events, setup) {
                                self.run_prompt(
                                    operation_id,
                                    prompt,
                                    record_prompt,
                                    inactivity_policy,
                                    end_report,
                                    events,
                                )
                                .await;
                                self.refresh_idle_mcp_if_owed().await;
                            } else {
                                // Setup never reached the owner, so the turn cannot have started.
                                end_report.record(PromptEndCause::StreamDropped);
                            }
                        }
                        Err(error) => {
                            let _ = accepted.send(Err(error));
                        }
                    }
                }
                RuntimeCommand::McpDesiredMaybeChanged => {
                    self.on_idle_mcp_desired_changed().await;
                }
                RuntimeCommand::AgentProcessReplaced { agent } => {
                    self.detach_replaced_agent(&agent);
                }
                RuntimeCommand::RespondToPermission { response, .. } => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                RuntimeCommand::Stop { response } => {
                    self.title_acquisition.close();
                    self.unload().await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                }
                RuntimeCommand::CancelActivePrompt => {}
                RuntimeCommand::Cancel {
                    operation_id,
                    completion,
                } => {
                    if let Some(completion) = completion {
                        let result = self.cleanup_outcome(operation_id);
                        let _ = completion.send(result);
                    }
                }
                RuntimeCommand::ClaimDirectProviderCall { response } => {
                    let _ = response.send(self.provider_session_id().to_string());
                }
                RuntimeCommand::AdoptUserTitle { title, response } => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                RuntimeCommand::TitlePoll { attempt } => {
                    self.run_title_poll(attempt).await;
                }
                RuntimeCommand::TitleUpdate { update } => {
                    self.observe_session_update(&update);
                }
            }
        }
    }

    /// Streams Ora's record to a client that opened this conversation.
    ///
    /// No provider call is made and no lifecycle state changes: a load is a read of the record Ora
    /// owns, and whether this session is attached is decided by prompts alone. An actor already
    /// holding a channel keeps it, and one holding none is not made to acquire one.
    async fn run_load(
        &mut self,
        events: mpsc::Sender<Result<LoadSessionEvent, RuntimeError>>,
        accepted: oneshot::Sender<Result<(), RuntimeError>>,
    ) {
        if accepted.send(Ok(())).is_err() {
            return;
        }
        // Ahead of the replay, and only while attached: these describe the provider serving this
        // conversation, so a client can read their presence as "this session has an agent to
        // configure" and their absence as "offer the agent's own catalog and let the next message
        // carry the choice". Not recorded — this is the session's present, not its transcript.
        if self.channel.is_some() && !self.reported_config_options.is_empty() {
            let update = SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
                self.reported_config_options.clone(),
            ));
            if events
                .send(Ok(LoadSessionEvent::session_update(update)))
                .await
                .is_err()
            {
                return;
            }
        }
        if let Replay::Delivered = self.replay_recorded_history(&events).await {
            let _ = events.send(Ok(LoadSessionEvent::Completed)).await;
        }
    }

    /// Marks the session degraded when a recording attempt just broke its history.
    pub(super) fn settle_record(&mut self, outcome: RecordOutcome) {
        let RecordOutcome::JustFailed { reason } = outcome else {
            return;
        };
        ora_debug!(
            session_id = %self.session.id,
            path = %self.recorder.path().display(),
            "session history stopped recording",
        );
        self.persist_session_history_state(HistoryState::Degraded { reason });
    }

    /// Handles controls arriving while a registered session has no active operation.
    async fn handle_idle_control(&mut self, control: Option<SessionControl>) {
        match control {
            Some(SessionControl::QueueOverflow) => {
                self.title_acquisition.close();
                self.unload().await;
            }
            Some(SessionControl::ConnectionLost(_)) | None => self.mark_stopped(),
        }
    }

    /// Completes an interrupted prompt request with the connection-level failure.
    fn fail_prompt(
        &mut self,
        events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
        error: RuntimeError,
    ) {
        let _ = events.try_send(Err(error));
        self.mark_stopped();
    }

    /// Drops the live registration when this session's agent process was replaced under it.
    ///
    /// The provider session died with the process, so the channel is dropped without `session/close`
    /// — that call would only ask a fresh agent to close an id it has never heard of. Detaching is
    /// enough to repair the session rather than end it: the next prompt attaches, which restores
    /// the session on the fresh process or rebuilds it when that fails. Without this the actor
    /// would keep a channel it believes is live and prompt against a session id the new process
    /// cannot resolve.
    fn detach_replaced_agent(&mut self, agent: &ora_domain::AgentRef) {
        if self.session.agent_ref != *agent || self.channel.is_none() {
            return;
        }
        ora_debug!(
            session_id = %self.session.id,
            agent = %agent,
            "detaching session after its agent process was replaced",
        );
        self.mark_stopped();
    }

    /// Names the provider session this actor is talking to right now.
    ///
    /// Usually the binding in the row, but a session rebuilt after one that could not be restored
    /// lives only in memory until the transcript is delivered. Reading it from here rather than
    /// from the row is what keeps a status write — which refreshes the snapshot from the database —
    /// from silently swapping the live identity back to the one being replaced.
    pub(super) fn provider_session_id(&self) -> &str {
        match &self.rebuilt_binding {
            Some(rebuilt) => &rebuilt.agent_session_id,
            None => &self.session.agent_session_id,
        }
    }

    /// Persists a stopped state after the provider session is detached or becomes unusable.
    fn mark_stopped(&mut self) {
        self.channel = None;
        self.reported_config_options.clear();
        self.title_acquisition.close();
        self.live_mcp = LiveMcpState::Inactive;
        self.persist_session_status(SessionStatus::Stopped);
        ora_debug!(session_id = %self.session.id, "session marked stopped");
    }

    /// Persists lifecycle status and refreshes the actor snapshot from the single-column result.
    pub(super) fn persist_session_status(&mut self, status: SessionStatus) {
        match self.repository.update_session_status(
            &self.session.id,
            status,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => self.session = session,
            Err(error) => ora_warn!(
                session_id = %self.session.id,
                error = %error,
                "failed to persist session lifecycle status",
            ),
        }
    }

    /// Persists history state without allowing an older actor snapshot to overwrite title fields.
    fn persist_session_history_state(&mut self, history_state: HistoryState) {
        match self.repository.update_session_history_state(
            &self.session.id,
            &history_state,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => self.session = session,
            Err(error) => ora_warn!(
                session_id = %self.session.id,
                error = %error,
                "failed to persist session history state",
            ),
        }
    }
}

#[cfg(test)]
impl<H: AgentRuntimeHost> Drop for RuntimeActor<H> {
    /// Signals the test harness after the actor has released all of its dependencies.
    fn drop(&mut self) {
        if let Some(exit_probe) = self.exit_probe.take() {
            let _ = exit_probe.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use crate::connection::ConnectionSupervisor;
    use crate::session_setup::AgentSessionBarriers;
    use crate::test_host::{InstalledAgents, RecordedEvents, TestHost};
    use crate::title_acquisition::TitleAcquisition;
    use crate::{MemorySessionStore, NoSessionMcp};
    use ora_domain::{
        AgentRef, AuditFields, PluginId, SessionId, SessionStatus, SessionTitle, WorkspaceId,
    };
    use ora_scheduler::Scheduler;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::timeout;

    /// Builds an attach that reports no installed package for these CLI-only supervisor tests.
    fn test_plugin_host() -> Arc<InstalledAgents> {
        Arc::new(InstalledAgents::default())
    }

    /// Names one installed agent package the supervisor fixtures bind their sessions to.
    ///
    /// The agent identity is the package's whole canonical plugin id, which is exactly what a
    /// session persists in `agent_cli`.
    fn test_agent_source() -> (AgentRef, PluginId) {
        (
            AgentRef::parse("official/ora-space.codex").expect("agent identity"),
            PluginId::new("official", "ora-space.codex").expect("plugin id"),
        )
    }

    /// Verifies dropping the manager's last sender lets the actor task terminate and release its dependencies.
    #[tokio::test]
    async fn actor_exits_after_command_sender_is_dropped() {
        let temporary = TempDir::new().expect("create actor test directory");
        let store = MemorySessionStore::default();
        let scheduler = Scheduler::new(chrono_tz::UTC);
        let (agent_ref, agent_source) = test_agent_source();
        let plugin_host = test_plugin_host();
        let connections = ConnectionSupervisors::<TestHost>::start(
            plugin_host.clone(),
            store.clone(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let connection = ConnectionSupervisor::start::<TestHost>(
            agent_ref.clone(),
            agent_source,
            Arc::new(Mutex::new(BTreeSet::new())),
            plugin_host.clone(),
            store.clone(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let recorder = crate::history::SessionRecorder::open(
            &temporary.path().join("sessions"),
            "session-1",
            0,
            &ora_domain::HistoryState::Writable,
            crate::test_host::RecordedEvents::default(),
            crate::history::LocalHistoryClock,
        )
        .expect("open actor recorder");
        let session = ora_domain::Session::new(
            SessionId::new("session-1"),
            WorkspaceId::new("workspace-1"),
            agent_ref,
            "provider-session-1",
            SessionStatus::Stopped,
            ora_domain::SessionMcpSelection::Automatic,
            AuditFields::new(0, 0, false),
        );
        let (commands, command_receiver) = mpsc::unbounded_channel();
        let command_sender = commands.downgrade();
        let (exit_sender, exit) = oneshot::channel();
        let actor = RuntimeActor {
            cleanup_outcomes: HashMap::new(),
            session,
            cwd: temporary.path().to_path_buf(),
            repository: store,
            clock: SystemClock,
            connection,
            channel: None,
            commands: command_receiver,
            recorder,
            sessions_root: temporary.path().join("sessions"),
            connections,
            handoff: HandoffDebt::Settled,
            rebuilt_binding: None,
            reported_config_options: Vec::new(),
            scheduler: scheduler.clone(),
            events: RecordedEvents::default(),
            title_acquisition: TitleAcquisition::disabled(),
            command_sender,
            session_mcp: NoSessionMcp::default(),
            barriers: Arc::new(AgentSessionBarriers::new()),
            live_mcp: LiveMcpState::Inactive,
            #[cfg(feature = "test-support")]
            test_prompt_inactivity_windows: None,
            exit_probe: Some(exit_sender),
        };
        let actor_task = tokio::spawn(actor.run());

        drop(commands);
        timeout(Duration::from_secs(1), exit)
            .await
            .expect("actor should drop after its command channel closes")
            .expect("actor drop probe should remain connected");
        actor_task.await.expect("actor task should exit cleanly");
        scheduler.shutdown().await;
    }

    /// Verifies a user-chosen title locks acquisition so a later agent title is ignored.
    #[tokio::test]
    async fn user_rename_locks_title_against_later_agent_updates() {
        let temporary = TempDir::new().expect("create actor test directory");
        let store = MemorySessionStore::default();
        let scheduler = Scheduler::new(chrono_tz::UTC);
        let (agent_ref, agent_source) = test_agent_source();
        let plugin_host = test_plugin_host();
        let connections = ConnectionSupervisors::<TestHost>::start(
            plugin_host.clone(),
            store.clone(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let connection = ConnectionSupervisor::start::<TestHost>(
            agent_ref.clone(),
            agent_source,
            Arc::new(Mutex::new(BTreeSet::new())),
            plugin_host.clone(),
            store.clone(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let recorder = crate::history::SessionRecorder::open(
            &temporary.path().join("sessions"),
            "session-1",
            0,
            &ora_domain::HistoryState::Writable,
            crate::test_host::RecordedEvents::default(),
            crate::history::LocalHistoryClock,
        )
        .expect("open actor recorder");
        let session = ora_domain::Session::new(
            SessionId::new("session-1"),
            WorkspaceId::new("workspace-1"),
            agent_ref,
            "provider-session-1",
            SessionStatus::Stopped,
            ora_domain::SessionMcpSelection::Automatic,
            AuditFields::new(0, 0, false),
        );
        let (commands, command_receiver) = mpsc::unbounded_channel();
        let command_sender = commands.downgrade();
        let mut actor = RuntimeActor {
            cleanup_outcomes: HashMap::new(),
            session,
            cwd: temporary.path().to_path_buf(),
            repository: store,
            clock: SystemClock,
            connection,
            channel: None,
            commands: command_receiver,
            recorder,
            sessions_root: temporary.path().join("sessions"),
            connections,
            handoff: HandoffDebt::Settled,
            rebuilt_binding: None,
            reported_config_options: Vec::new(),
            scheduler: scheduler.clone(),
            events: RecordedEvents::default(),
            title_acquisition: TitleAcquisition::awaiting_first_prompt(true),
            command_sender,
            session_mcp: NoSessionMcp::default(),
            barriers: Arc::new(AgentSessionBarriers::new()),
            live_mcp: LiveMcpState::Inactive,
            #[cfg(feature = "test-support")]
            test_prompt_inactivity_windows: None,
            exit_probe: None,
        };
        let user_title = SessionTitle::parse("User title").expect("valid user title");
        actor.adopt_user_title(user_title.clone());
        actor.persist_agent_title("Agent title");
        assert_eq!(actor.session.title.as_ref(), Some(&user_title));
        assert!(!actor.title_acquisition.accepts_title());
        drop(commands);
        drop(actor);
        scheduler.shutdown().await;
    }
}

/// Sends one attach's setup updates ahead of its turn, reporting whether a client is still there.
///
/// A closed queue means the caller abandoned the send before it began, so the turn is dropped
/// rather than started for nobody.
fn publish_setup(
    events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    setup: Vec<SessionUpdate>,
) -> bool {
    setup.into_iter().all(|update| {
        events
            .try_send(Ok(PromptSessionEvent::SessionUpdate {
                update,
                tool_timing: None,
            }))
            .is_ok()
    })
}

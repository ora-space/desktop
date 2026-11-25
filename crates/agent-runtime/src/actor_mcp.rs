//! Session MCP setup, live refresh, and prompt admission for one runtime actor.

use super::super::RuntimeCommand;
use super::super::SessionChannel;
use super::super::events::settle_idle_event;
use super::super::routing::SessionEvent;
use super::super::scheduling::{ActiveInput, ActiveInputState};
use super::super::start::record_session_mcp_boundary;
use super::super::support::{
    agent_timed_out, contract_session, map_acp_error, mcp_load_unsupported, protocol_violation,
    runtime_unavailable, session_event_overflow, session_stopped,
};
use super::{RuntimeActor, permission_not_pending, session_busy, session_setup_window};
use crate::AgentRuntimeHost;
use crate::RuntimeError;
use crate::host::SessionSetup;
use crate::host::SkillCatalogSource;
use crate::session_setup::{
    AgentSessionMcpCapabilities, BarrierGuard, BarrierReason, LiveMcpEvent, LiveMcpPromptAdmission,
    SessionMcpSnapshot,
};
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{
    LoadSessionRequest as AcpLoadSessionRequest, LoadSessionResponse, RequestPermissionOutcome,
    RequestPermissionResponse,
};
use ora_contracts::StopSessionResponse;
use ora_logging::{ora_debug, ora_warn};
use std::collections::HashMap;
use tokio::time::Instant;

impl<H: AgentRuntimeHost> crate::AgentRuntimeManager<H> {
    /// Wakes every Live Session so it re-reads the current Desired MCP revision.
    ///
    /// The notification is level-triggered and secret-free. Stopped Sessions ignore it; idle
    /// Sessions refresh immediately; busy Sessions mark refresh as owed work.
    pub fn notify_mcp_desired_changed(&self) {
        let Ok(actors) = self.inner.actors.read() else {
            return;
        };
        for handle in actors.values() {
            let _ = handle.commands.send(RuntimeCommand::McpDesiredMaybeChanged);
        }
    }
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Only hosts with an exclusive process can ask to rotate it before a load.
    pub(crate) async fn replace_mcp_process_if_needed(
        &mut self,
        snapshot: &SessionMcpSnapshot,
    ) -> Result<bool, RuntimeError> {
        if !self
            .session_mcp
            .mcp_replacement_required(snapshot.revision())
        {
            return Ok(false);
        }
        let plugin = self
            .session_mcp
            .plugin_id_for_agent(&self.session.agent_ref)
            .ok_or_else(runtime_unavailable)?;
        self.connections.suspend_plugin_agent(&plugin.to_string());
        self.channel = None;
        self.session_mcp.mcp_detached();
        // A failure here leaves the live state to `attach_provider_session`, which blocks the
        // revision it attempted.
        self.session_mcp
            .replace_after_retirement(self.connections.retire_mcp_agent(&plugin.to_string()))
            .await?;
        self.connections.resume_plugin_agent(&plugin.to_string());
        self.connections.sync_plugin_agents();
        self.connection = self.connections.for_agent(&self.session.agent_ref)?;
        if let Err(error) = self.connection.ready().await {
            self.session_mcp.mcp_failed(&error);
            return Err(error.during_mcp());
        }
        Ok(true)
    }
    /// Observes the latest Desired MCP revision without sending ACP frames.
    pub(super) fn note_desired_mcp(&mut self) {
        if let Err(error) = self.observe_desired_mcp() {
            ora_warn!(
                session_id = %self.session.id,
                error = %error,
                "failed to observe Desired MCP revision"
            );
        }
    }

    /// Re-reads Desired MCP and reports whether an idle Session owes an immediate refresh.
    fn observe_desired_mcp(&mut self) -> Result<bool, RuntimeError> {
        let desired = self.session_mcp.desired_mcp_revision()?;
        let (state, refresh_now) = self
            .live_mcp
            .on_event(LiveMcpEvent::DesiredObserved(desired));
        self.live_mcp = state;
        Ok(refresh_now)
    }

    /// Idle path: re-read Desired MCP and refresh immediately when the Session is live.
    pub(super) async fn on_idle_mcp_desired_changed(&mut self) {
        match self.observe_desired_mcp() {
            Ok(true) if self.channel.is_some() || self.session_mcp.preserve_provider_identity() => {
                let result = if self.channel.is_none() {
                    self.attach_provider_session(None).await.map(drop)
                } else {
                    self.refresh_mcp().await
                };
                if let Err(error) = result {
                    ora_warn!(
                        session_id = %self.session.id,
                        error = %error,
                        "idle MCP refresh failed"
                    );
                }
            }
            Ok(_) => {}
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    error = %error,
                    "failed to observe Desired MCP revision"
                );
            }
        }
    }

    /// Prompt admission: refresh first when Active is missing, stale, in-flight, or blocked.
    pub(crate) async fn ensure_current_mcp(&mut self) -> Result<(), RuntimeError> {
        let desired = self.session_mcp.desired_mcp_revision()?;
        match self.live_mcp.prompt_admission(&desired) {
            LiveMcpPromptAdmission::Admit => Ok(()),
            LiveMcpPromptAdmission::RefreshFirst { .. } => self.refresh_mcp().await,
        }
    }

    /// Refreshes an idle Session after a prompt if Desired changed while it was busy.
    pub(super) async fn refresh_idle_mcp_if_owed(&mut self) {
        if self.channel.is_none() {
            return;
        }
        match self.observe_desired_mcp() {
            Ok(true) => {
                if let Err(error) = self.refresh_mcp().await {
                    ora_warn!(
                        session_id = %self.session.id,
                        error = %error,
                        "post-prompt MCP refresh failed"
                    );
                }
            }
            Ok(false) => {}
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    error = %error,
                    "failed to observe Desired MCP revision after prompt"
                );
            }
        }
    }

    /// Resolves one Snapshot with the current connection's advertised MCP capabilities.
    pub(crate) fn resolve_mcp_snapshot(&self) -> Result<SessionMcpSnapshot, RuntimeError> {
        let connection = self.connection.current()?;
        self.session_mcp.resolve_mcp(
            &self.cwd,
            AgentSessionMcpCapabilities::new(
                connection.load_session_supported,
                connection.http_mcp_supported,
            ),
        )
    }

    /// Converges this live Session onto the current Snapshot, holding the shared Agent barrier.
    async fn refresh_mcp(&mut self) -> Result<(), RuntimeError> {
        let recovered = self
            .connections
            .recover_mcp_replacement(&self.session.agent_ref, &self.session_mcp)
            .await?;
        self.connection = self.connections.for_agent(&self.session.agent_ref)?;
        loop {
            let snapshot = self.resolve_mcp_snapshot()?;
            if recovered
                && self
                    .session_mcp
                    .mcp_replacement_required(snapshot.revision())
            {
                let error = RuntimeError::mcp_retry_required();
                self.session_mcp.mcp_failed(&error);
                return Err(error);
            }
            if self
                .session_mcp
                .mcp_replacement_required(snapshot.revision())
            {
                // Attach owns the single replacement and restore. A new target cannot cause a
                // second replacement in this refresh loop; its notification owns the next retry.
                self.attach_provider_session(None).await?;
                return Ok(());
            }
            let revision = snapshot.revision().clone();
            if self.live_mcp.is_current(&revision) {
                self.session_mcp.mcp_loaded(&revision);
                return Ok(());
            }
            let _barrier = self.acquire_mcp_barrier().await;
            let snapshot = self.resolve_mcp_snapshot()?;
            let revision = snapshot.revision().clone();
            if self.live_mcp.is_current(&revision) {
                self.session_mcp.mcp_loaded(&revision);
                return Ok(());
            }
            self.live_mcp = self
                .live_mcp
                .on_event(LiveMcpEvent::RefreshStarted(revision.clone()))
                .0;
            match self.reload_provider_mcp(snapshot).await {
                Ok(()) => {
                    self.session_mcp.mcp_loaded(&revision);
                    let (state, more) = self
                        .live_mcp
                        .on_event(LiveMcpEvent::RefreshSucceeded(revision));
                    self.live_mcp = state;
                    if !more {
                        return Ok(());
                    }
                }
                Err(error) => {
                    self.session_mcp.mcp_failed(&error);
                    self.live_mcp = self
                        .live_mcp
                        .on_event(LiveMcpEvent::RefreshFailed {
                            requested: revision,
                        })
                        .0;
                    return Err(error.during_mcp());
                }
            }
        }
    }

    /// Waits for the Agent Session Barrier to serialize live MCP refreshes for this agent plugin.
    async fn acquire_mcp_barrier(&self) -> Option<BarrierGuard> {
        let plugin_id = self
            .session_mcp
            .plugin_id_for_agent(&self.session.agent_ref)?;
        let guard = self
            .barriers
            .for_plugin(&plugin_id)
            .acquire(BarrierReason::McpRefresh)
            .await;
        ora_debug!(
            session_id = %self.session.id,
            reason = ?guard.reason(),
            "acquired Agent Session Barrier for MCP refresh"
        );
        Some(guard)
    }

    /// Sends one live `session/load` carrying MCP and discards the agent's own replay.
    async fn reload_provider_mcp(
        &mut self,
        snapshot: SessionMcpSnapshot,
    ) -> Result<(), RuntimeError> {
        let Some(mut channel) = self.channel.take() else {
            if self.session_mcp.preserve_provider_identity() {
                self.session_mcp.mcp_detached();
            } else {
                self.live_mcp = self.live_mcp.on_event(LiveMcpEvent::Detached).0;
            }
            return Err(session_stopped());
        };
        if !snapshot.revision().is_empty() && !channel.connection.load_session_supported {
            self.isolate_channel(channel).await;
            return Err(mcp_load_unsupported());
        }
        if !channel.connection.load_session_supported {
            self.channel = Some(channel);
            return Ok(());
        }
        let client = channel.connection.client.clone();
        // The live identity, not the row's: a session rebuilt for this turn is not persisted until
        // the transcript reaches it, and refreshing MCP on the replaced id would reach nothing.
        let agent_session_id = self.provider_session_id().to_string();
        // The refresh reconnects the delivered MCP servers before responding, so its inactivity
        // window widens with the snapshot just like `session/new` does.
        let window = session_setup_window(&snapshot);
        let mut servers = snapshot.servers().to_vec();
        servers.extend(self.session_mcp.host_mcp_servers());
        let skill_entries = match self.skill_catalog.skill_catalog(&self.session.workspace_id) {
            Ok(snapshot) => snapshot.entries,
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    workspace_id = %self.session.workspace_id,
                    error = %error,
                    "failed to read skill catalog for MCP refresh session/load; omitting skill directory metadata",
                );
                Vec::new()
            }
        };
        let request = crate::skill_catalog::apply_skill_setup(
            AcpLoadSessionRequest::new(AcpSessionId::new(agent_session_id.clone()), &self.cwd)
                .mcp_servers(servers),
            channel.connection.additional_directories_supported,
            self.skill_catalog.extra_read_roots(),
            &skill_entries,
        );
        record_session_mcp_boundary(
            &self.session_mcp,
            &self.session.id,
            &self.session.agent_ref,
            Some(&agent_session_id),
            AGENT_METHOD_NAMES.session_load,
            &snapshot,
            &self.cwd,
        );
        ora_debug!(session_id = %self.session.id, "session/load MCP refresh sent");
        let pending = match client
            .start_session_request::<_, LoadSessionResponse>(
                AcpSessionId::new(agent_session_id),
                AGENT_METHOD_NAMES.session_load,
                &request,
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                self.isolate_channel(channel).await;
                return Err(map_acp_error(error));
            }
        };
        let deadline = tokio::time::sleep(window);
        tokio::pin!(deadline);
        let mut input_state = ActiveInputState::default();
        loop {
            let input = tokio::select! {
                biased;
                input = input_state.recv(
                    &mut channel.events,
                    &mut channel.controls,
                    &mut self.commands,
                ) => input,
                _ = &mut deadline => {
                    self.cancel(&client, &HashMap::new()).await;
                    self.isolate_channel(channel).await;
                    return Err(agent_timed_out("agent CLI session load timed out"));
                }
            };
            match input {
                ActiveInput::Event(SessionEvent::Update(update, arrived)) => {
                    channel.retain_commands(&update);
                    self.observe_replayed_session_update(&update.update);
                    deadline.as_mut().reset(Instant::now() + window);
                    settle_idle_event(&client, SessionEvent::Update(update, arrived)).await;
                }
                ActiveInput::Event(SessionEvent::Permission(permission)) => {
                    let _ = client
                        .respond(
                            &permission.request_id,
                            &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                        )
                        .await;
                    self.isolate_channel(channel).await;
                    return Err(protocol_violation(
                        "permission request during session/load is unsupported",
                    ));
                }
                ActiveInput::Event(SessionEvent::Response(response)) => {
                    if !pending.matches_response(&response) {
                        continue;
                    }
                    return match pending.finish(response) {
                        Ok(_) => {
                            ora_debug!(session_id = %self.session.id, "session/load MCP refresh completed");
                            self.channel = Some(channel);
                            Ok(())
                        }
                        Err(error) => {
                            self.isolate_channel(channel).await;
                            Err(map_acp_error(error))
                        }
                    };
                }
                ActiveInput::Control(super::routing::SessionControl::ConnectionLost(error)) => {
                    self.fail_prompt_channel(channel);
                    return Err(error);
                }
                ActiveInput::Control(super::routing::SessionControl::QueueOverflow) => {
                    self.isolate_channel(channel).await;
                    return Err(session_event_overflow("session event queue overflowed"));
                }
                ActiveInput::EventsClosed | ActiveInput::ControlsClosed => {
                    self.isolate_channel(channel).await;
                    return Err(runtime_unavailable());
                }
                ActiveInput::Command(RuntimeCommand::McpDesiredMaybeChanged) => {
                    self.note_desired_mcp();
                }
                ActiveInput::Command(RuntimeCommand::Prompt { accepted, .. })
                | ActiveInput::Command(RuntimeCommand::Load { accepted, .. }) => {
                    let _ = accepted.send(Err(session_busy()));
                }
                ActiveInput::Command(RuntimeCommand::Stop { response }) => {
                    self.cancel(&client, &HashMap::new()).await;
                    self.isolate_channel(channel).await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                    return Err(session_stopped());
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { response, .. }) => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                ActiveInput::Command(RuntimeCommand::ClaimDirectProviderCall { response }) => {
                    let _ = response.send(self.provider_session_id().to_string());
                }
                ActiveInput::Command(RuntimeCommand::AdoptConfigOptions {
                    agent_session_id,
                    config_options,
                    response,
                }) => {
                    self.adopt_config_options(&agent_session_id, config_options);
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::AdoptUserTitle { title, response }) => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::Cancel {
                    operation_id,
                    completion,
                }) => {
                    if let Some(completion) = completion {
                        let _ = completion.send(self.cleanup_outcome(operation_id));
                    }
                }
                ActiveInput::Command(RuntimeCommand::CancelActivePrompt)
                | ActiveInput::Command(RuntimeCommand::TitlePoll { .. }) => {}
                // A refresh owns the channel's events, so quiet cannot be shown from here.
                ActiveInput::Command(RuntimeCommand::TakeIdleTurnTraffic { response }) => {
                    let _ = response.send(true);
                }
                ActiveInput::Command(RuntimeCommand::TitleUpdate { update }) => {
                    self.observe_session_update(&update);
                }
                ActiveInput::CommandsClosed => {
                    self.isolate_channel(channel).await;
                    return Err(runtime_unavailable());
                }
            }
        }
    }

    /// Records that a live refresh lost its channel without a separate isolation path.
    fn fail_prompt_channel(&mut self, _channel: SessionChannel) {
        self.mark_stopped();
    }

    /// Marks the Snapshot that was successfully loaded as this Session's Active revision.
    pub(crate) fn record_loaded_mcp(&mut self, snapshot: &SessionMcpSnapshot) {
        let revision = snapshot.revision().clone();
        self.session_mcp.mcp_loaded(&revision);
        self.live_mcp = self
            .live_mcp
            .on_event(LiveMcpEvent::RefreshStarted(revision.clone()))
            .0;
        let (state, more) = self
            .live_mcp
            .on_event(LiveMcpEvent::RefreshSucceeded(revision));
        self.live_mcp = state;
        if more {
            self.note_desired_mcp();
        }
    }
}

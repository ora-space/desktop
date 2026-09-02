//! Session MCP setup, live refresh, and prompt admission for one runtime actor.

use super::super::RuntimeCommand;
use super::super::SESSION_SETUP_TIMEOUT;
use super::super::SessionChannel;
use super::super::events::settle_idle_event;
use super::super::routing::SessionEvent;
use super::super::scheduling::{ActiveInput, ActiveInputState};
use super::super::support::{
    contract_session, map_acp_error, runtime_internal, runtime_unavailable, session_stopped,
};
use super::{RuntimeActor, permission_not_pending, session_busy};
use crate::BackendError;
use crate::session_setup::{
    AgentSessionMcpCapabilities, BarrierGuard, BarrierReason, LiveMcpEvent, LiveMcpPromptAdmission,
    SessionMcpError, SessionMcpSnapshot, SessionSetup,
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

impl RuntimeActor {
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
    fn observe_desired_mcp(&mut self) -> Result<bool, BackendError> {
        let desired = crate::session_setup::resolve_session_mcp_revision(
            &self.session_mcp,
            &self.session_mcp,
        )
        .map_err(SessionMcpError::into_backend)?;
        let (state, refresh_now) = self
            .live_mcp
            .on_event(LiveMcpEvent::DesiredObserved(desired));
        self.live_mcp = state;
        Ok(refresh_now)
    }

    /// Idle path: re-read Desired MCP and refresh immediately when the Session is live.
    pub(super) async fn on_idle_mcp_desired_changed(&mut self) {
        match self.observe_desired_mcp() {
            Ok(true) if self.channel.is_some() => {
                if let Err(error) = self.refresh_mcp().await {
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
    pub(super) async fn ensure_current_mcp(&mut self) -> Result<(), BackendError> {
        let desired = crate::session_setup::resolve_session_mcp_revision(
            &self.session_mcp,
            &self.session_mcp,
        )
        .map_err(SessionMcpError::into_backend)?;
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
    fn resolve_mcp_snapshot(&self) -> Result<SessionMcpSnapshot, BackendError> {
        let connection = self.connection.current()?;
        SessionSetup::resolve(
            &self.session_mcp,
            &self.cwd,
            AgentSessionMcpCapabilities::new(
                connection.load_session_supported,
                connection.http_mcp_supported,
            ),
        )
        .map(|setup| setup.mcp)
        .map_err(SessionMcpError::into_backend)
    }

    /// Converges this live Session onto the current Snapshot, holding the shared Agent barrier.
    async fn refresh_mcp(&mut self) -> Result<(), BackendError> {
        loop {
            let snapshot = self.resolve_mcp_snapshot()?;
            let revision = snapshot.revision().clone();
            if self.live_mcp.is_current(&revision) {
                return Ok(());
            }
            let _barrier = self.acquire_mcp_barrier().await;
            let snapshot = self.resolve_mcp_snapshot()?;
            let revision = snapshot.revision().clone();
            if self.live_mcp.is_current(&revision) {
                return Ok(());
            }
            self.live_mcp = self
                .live_mcp
                .on_event(LiveMcpEvent::RefreshStarted(revision.clone()))
                .0;
            match self.reload_provider_mcp(snapshot).await {
                Ok(()) => {
                    let (state, more) = self
                        .live_mcp
                        .on_event(LiveMcpEvent::RefreshSucceeded(revision));
                    self.live_mcp = state;
                    if !more {
                        return Ok(());
                    }
                }
                Err(error) => {
                    self.live_mcp = self
                        .live_mcp
                        .on_event(LiveMcpEvent::RefreshFailed {
                            requested: revision,
                        })
                        .0;
                    return Err(error);
                }
            }
        }
    }

    /// Waits for the Agent Session Barrier so MCP refresh cannot race Effect mutation.
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

    /// Builds the ACP `session/load` used by both explicit restore and live MCP refresh.
    fn mcp_load_request(&self, snapshot: &SessionMcpSnapshot) -> AcpLoadSessionRequest {
        AcpLoadSessionRequest::new(
            AcpSessionId::new(self.session.agent_session_id.clone()),
            &self.cwd,
        )
        .mcp_servers(snapshot.servers().to_vec())
    }

    /// Sends one live `session/load` carrying MCP and discards the agent's own replay.
    async fn reload_provider_mcp(
        &mut self,
        snapshot: SessionMcpSnapshot,
    ) -> Result<(), BackendError> {
        let Some(mut channel) = self.channel.take() else {
            self.live_mcp = self.live_mcp.on_event(LiveMcpEvent::Detached).0;
            return Err(session_stopped());
        };
        if !snapshot.revision().is_empty() && !channel.connection.load_session_supported {
            self.isolate_channel(channel).await;
            return Err(SessionMcpError::LoadCapabilityMissing.into_backend());
        }
        if !channel.connection.load_session_supported {
            self.channel = Some(channel);
            return Ok(());
        }
        let client = channel.connection.client.clone();
        let request = self.mcp_load_request(&snapshot);
        ora_debug!(session_id = %self.session.id, "session/load MCP refresh sent");
        let pending = match client
            .start_session_request::<_, LoadSessionResponse>(
                AcpSessionId::new(self.session.agent_session_id.clone()),
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
        let deadline = tokio::time::sleep(SESSION_SETUP_TIMEOUT);
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
                    return Err(runtime_internal(
                        "agent_load_timeout",
                        "agent CLI session load timed out",
                    ));
                }
            };
            match input {
                ActiveInput::Event(SessionEvent::Update(update)) => {
                    self.observe_session_update(&update.update);
                    deadline
                        .as_mut()
                        .reset(Instant::now() + SESSION_SETUP_TIMEOUT);
                    settle_idle_event(&client, SessionEvent::Update(update)).await;
                }
                ActiveInput::Event(SessionEvent::Permission(permission)) => {
                    let _ = client
                        .respond(
                            &permission.request_id,
                            &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                        )
                        .await;
                    self.isolate_channel(channel).await;
                    return Err(runtime_internal(
                        "agent_protocol_error",
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
                    return Err(runtime_internal(
                        "agent_event_overflow",
                        "session event queue overflowed",
                    ));
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
                ActiveInput::Command(RuntimeCommand::AgentProcessReplaced { agent }) => {
                    self.channel = Some(channel);
                    self.detach_replaced_agent(&agent);
                    return Err(session_stopped());
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { response, .. }) => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                ActiveInput::Command(RuntimeCommand::PreemptTitlePolling { response }) => {
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::AdoptUserTitle { title, response }) => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::Cancel { .. })
                | ActiveInput::Command(RuntimeCommand::CancelActivePrompt)
                | ActiveInput::Command(RuntimeCommand::TitlePoll { .. }) => {}
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

    /// Resolves the Snapshot that an explicit `session/load` must send with the restore.
    pub(super) fn resolve_load_mcp_snapshot(&self) -> Result<SessionMcpSnapshot, BackendError> {
        self.resolve_mcp_snapshot()
    }

    /// Marks the Snapshot that was successfully loaded as this Session's Active revision.
    pub(super) fn record_loaded_mcp(&mut self, snapshot: &SessionMcpSnapshot) {
        let revision = snapshot.revision().clone();
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

//! Attaches one session to a provider at the moment a prompt needs one.
//!
//! Opening a conversation reads Ora's record and stops there, so every session an actor holds may
//! be unattached. The first prompt is what has to reach an agent, and this is where that happens:
//! restore the provider session the binding names, or, when it cannot be restored, build a new one
//! and let the transcript Ora owns carry the context across.

use super::handoff::HandoffDebt;
use super::routing::{SessionChannel, SessionControl, SessionEvent};
use super::start::{
    PendingProviderSession, ProviderSessionRelease, apply_model_intent, create_provider_session,
};
use super::support::{map_acp_error, runtime_internal};
use super::{RuntimeActor, SESSION_SETUP_TIMEOUT};
use crate::BackendError;
use crate::session_setup::{LiveMcpState, SessionMcpSnapshot};
use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate,
    LoadSessionRequest as AcpLoadSessionRequest, LoadSessionResponse, RequestPermissionOutcome,
    RequestPermissionResponse, SessionConfigOption, SessionId as AcpSessionId, SessionUpdate,
};
use ora_application::{Clock, SessionRepository};
use ora_domain::SessionStatus;
use ora_logging::{ora_debug, ora_warn};
use std::collections::HashMap;
use tokio::time::{Instant, sleep};

/// A provider session Ora created to replace one it could not restore, before the row knows.
///
/// The binding is deliberately not persisted here. A crash between building this session and
/// delivering the transcript would otherwise leave the row pointing at a provider session that
/// holds nothing, and no record says a handoff is owed — the conversation would silently continue
/// against an agent with no context. Keeping the old id in the row instead makes that same crash
/// self-repairing: the next prompt fails to restore it exactly as this one did, rebuilds, and
/// delivers the transcript again.
pub(super) struct RebuiltBinding {
    pub(super) agent_session_id: String,
    /// Releases the provider session unless the delivery that earns it commits.
    release: ProviderSessionRelease,
}

impl RuntimeActor {
    /// Ensures this session holds a live provider channel carrying the MCP set configured now.
    ///
    /// Returns the setup this attach observed, as the session updates a prompt stream can carry —
    /// empty when the session was already attached and nothing new was reported.
    ///
    /// The MCP check runs either way. An attach resolved its Snapshot moments earlier, but the
    /// configured set can change between that handshake and admission, and a session that was
    /// already attached has not been asked at all since its last turn.
    ///
    /// `model` is the choice a picker had nowhere to put while this session held no provider. It
    /// belongs to acquiring one, so it is consumed by the attach and ignored when none happens: a
    /// session already holding a channel reports its own options and is configured through them.
    pub(super) async fn ensure_attached(
        &mut self,
        model: Option<&str>,
    ) -> Result<Vec<SessionUpdate>, BackendError> {
        let setup = match self.channel {
            Some(_) => Vec::new(),
            None => self.attach_provider_session(model).await?,
        };
        self.ensure_current_mcp().await?;
        Ok(setup)
    }

    /// Acquires a provider channel for a session holding none, rebuilding when the old is unusable.
    async fn attach_provider_session(
        &mut self,
        model: Option<&str>,
    ) -> Result<Vec<SessionUpdate>, BackendError> {
        let channel = self
            .connection
            .open_session_channel(self.provider_session_id(), self.session.id.as_ref())?;
        let restored = if channel.connection.load_session_supported {
            // Resolved before the restore because `session/load` is the frame that carries MCP: an
            // agent restored without it would serve the turn against a stale server set. A Snapshot
            // that cannot be resolved fails the send rather than attaching without one.
            let snapshot = self.resolve_mcp_snapshot()?;
            match self.restore_provider_session(channel, &snapshot).await {
                Ok(config_options) => {
                    self.record_loaded_mcp(&snapshot);
                    // Applied against what this restore reported rather than the catalog the
                    // picker offered: only the agent can say whether it still serves that model.
                    Some(self.apply_restored_model(model, config_options).await)
                }
                Err(error) => {
                    // An agent that cannot restore this session is not a failed send: the
                    // conversation is Ora's, and a fresh provider session can be told it.
                    ora_debug!(
                        session_id = %self.session.id,
                        error = %error,
                        "provider session could not be restored; rebuilding it",
                    );
                    None
                }
            }
        } else {
            drop(channel);
            ora_debug!(
                session_id = %self.session.id,
                "agent does not support session/load; rebuilding the provider session",
            );
            None
        };
        match restored {
            Some(config_options) => {
                self.persist_session_status(SessionStatus::Running);
                self.reported_config_options = config_options.clone();
                Ok(setup_updates(config_options, Vec::new()))
            }
            None => self.rebuild_provider_session(model).await,
        }
    }

    /// Restores the provider session this binding names, keeping its channel on success.
    ///
    /// `session/load` is sent so the agent recovers the context it needs to answer the next
    /// prompt. Everything it recites on the way is discarded: what the user is shown came from
    /// Ora's record when the conversation was opened.
    async fn restore_provider_session(
        &mut self,
        mut channel: SessionChannel,
        snapshot: &SessionMcpSnapshot,
    ) -> Result<Vec<SessionConfigOption>, BackendError> {
        let client = channel.connection.client.clone();
        let agent_session_id = self.provider_session_id().to_string();
        let request =
            AcpLoadSessionRequest::new(AcpSessionId::new(agent_session_id.clone()), &self.cwd)
                .mcp_servers(snapshot.servers().to_vec());
        ora_debug!(session_id = %self.session.id, "session/load sent");
        let pending = client
            .start_session_request::<_, LoadSessionResponse>(
                AcpSessionId::new(agent_session_id),
                AGENT_METHOD_NAMES.session_load,
                &request,
            )
            .await
            .map_err(map_acp_error)?;
        let deadline = sleep(SESSION_SETUP_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                // A response already queued behind earlier events is more useful than a deadline
                // that became ready at the same time.
                biased;
                event = channel.events.recv() => match event {
                    Some(SessionEvent::Update(update)) => {
                        // The agent is reciting history Ora already owns. Draining it keeps the
                        // queue clear and proves the provider is still working.
                        self.observe_session_update(&update.update);
                        deadline.as_mut().reset(Instant::now() + SESSION_SETUP_TIMEOUT);
                    }
                    Some(SessionEvent::Permission(permission)) => {
                        let _ = client
                            .respond(
                                &permission.request_id,
                                &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                            )
                            .await;
                        return Err(runtime_internal(
                            "agent_protocol_error",
                            "permission request during session/load is unsupported",
                        ));
                    }
                    Some(SessionEvent::Response(response)) => {
                        if !pending.matches_response(&response) {
                            continue;
                        }
                        let response = pending.finish(response).map_err(map_acp_error)?;
                        ora_debug!(session_id = %self.session.id, "session/load completed");
                        self.channel = Some(channel);
                        return Ok(response.config_options.unwrap_or_default());
                    }
                    None => return Err(super::support::runtime_unavailable()),
                },
                control = channel.controls.recv() => match control {
                    Some(SessionControl::ConnectionLost(error)) => return Err(error),
                    Some(SessionControl::QueueOverflow) => {
                        return Err(runtime_internal(
                            "agent_event_overflow",
                            "session event queue overflowed",
                        ));
                    }
                    None => return Err(super::support::runtime_unavailable()),
                },
                () = &mut deadline => {
                    ora_debug!(session_id = %self.session.id, "session/load timed out");
                    self.cancel(&client, &HashMap::new()).await;
                    return Err(runtime_internal(
                        "agent_load_timeout",
                        "agent CLI session load timed out",
                    ));
                }
            }
        }
    }

    /// Creates a replacement provider session and owes it the conversation.
    ///
    /// The debt is only raised when none is outstanding: a switch that has not delivered its
    /// transcript yet already owes the same conversation and already keeps its own books.
    async fn rebuild_provider_session(
        &mut self,
        model: Option<&str>,
    ) -> Result<Vec<SessionUpdate>, BackendError> {
        let PendingProviderSession {
            release,
            agent_session_id,
            channel,
            available_commands,
            config_options,
            mcp_revision,
            ..
        } = create_provider_session(
            &self.connections,
            &self.session_mcp,
            &self.session.id,
            &self.session.agent_ref,
            &self.cwd,
            model,
        )
        .await?;
        ora_debug!(
            session_id = %self.session.id,
            agent_session_id,
            "provider session rebuilt under the same agent",
        );
        self.rebuilt_binding = Some(RebuiltBinding {
            agent_session_id,
            release,
        });
        if let HandoffDebt::Settled = self.handoff {
            self.handoff = HandoffDebt::Ephemeral;
        }
        self.channel = Some(channel);
        // `session/new` carried the current Snapshot, so the replacement starts Active on it
        // rather than owing the refresh that an attach through `session/load` has to record.
        self.live_mcp = LiveMcpState::Active(mcp_revision);
        self.persist_session_status(SessionStatus::Running);
        self.reported_config_options = config_options.clone();
        Ok(setup_updates(config_options, available_commands))
    }

    /// Applies a pending model choice to the provider session this attach just restored.
    ///
    /// `session/new` applies its caller's intent inside the handshake, so only the restore path
    /// needs this. Nothing is owed when no choice was pending, which is every prompt after the
    /// first one a reopened conversation sends.
    async fn apply_restored_model(
        &self,
        model: Option<&str>,
        config_options: Vec<SessionConfigOption>,
    ) -> Vec<SessionConfigOption> {
        let Some(model) = model else {
            return config_options;
        };
        apply_model_intent(
            &self.connections,
            &self.session.agent_ref,
            self.provider_session_id(),
            model,
            config_options,
        )
        .await
    }

    /// Persists a rebuilt binding once the prompt that carried the transcript was accepted.
    ///
    /// The binding is only released to the row on a successful write. A failed one leaves it in
    /// memory, which keeps the identity every later frame uses pointing at the session actually
    /// serving this turn, and leaves its cleanup with the guard for an actor that ends before any
    /// row names it.
    pub(super) fn commit_rebuilt_binding(&mut self) {
        let Some(agent_session_id) = self
            .rebuilt_binding
            .as_ref()
            .map(|rebuilt| rebuilt.agent_session_id.clone())
        else {
            return;
        };
        match self.repository.update_session_binding(
            &self.session.id,
            self.session.agent_ref.clone(),
            &agent_session_id,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => {
                self.session = session;
                if let Some(rebuilt) = self.rebuilt_binding.take() {
                    rebuilt.release.commit();
                }
            }
            Err(error) => ora_warn!(
                session_id = %self.session.id,
                error = %error,
                "failed to persist a rebuilt provider binding; keeping it in memory",
            ),
        }
    }
}

/// Renders what one handshake reported as the updates a prompt stream can carry ahead of its turn.
///
/// Both are session-scoped rather than part of the turn, and neither is recorded: they describe
/// the provider serving this conversation right now, not what was said in it.
fn setup_updates(
    config_options: Vec<SessionConfigOption>,
    available_commands: Vec<AvailableCommand>,
) -> Vec<SessionUpdate> {
    let mut updates = Vec::new();
    if !config_options.is_empty() {
        updates.push(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
            config_options,
        )));
    }
    if !available_commands.is_empty() {
        updates.push(SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(available_commands),
        ));
    }
    updates
}

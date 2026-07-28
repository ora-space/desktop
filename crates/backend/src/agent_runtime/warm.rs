//! Drives warm provider sessions between the pool's decisions and ACP.
//!
//! [`WarmPool`] decides what should happen; this module performs it. Nothing
//! here holds the runtime's lifecycle lock, so opening a chat surface never
//! serializes against prompts running in other sessions.

use super::collect_setup_commands;
use super::connection::ConnectionSupervisors;
use super::support::{map_acp_error, runtime_internal};
use super::warm_pool::{
    ConfigTarget, CreatedProvider, RebuildPlan, ReleasedSession, WarmDecision, WarmKey, WarmPool,
};
use crate::BackendError;
use crate::clock::SystemClock;
use ora_application::{Clock, SessionIdGenerator, UuidSessionIdGenerator};
use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
use ora_contracts::acp::session::{
    CloseSessionRequest, CloseSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    NewSessionRequest, NewSessionResponse,
};
use ora_contracts::acp::session_config_options::{
    SessionConfigId, SessionConfigOption, SessionConfigOptionValue, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse,
};
use ora_contracts::acp::slash_command::AvailableCommand;
use ora_domain::{AgentCli, SessionId};
use ora_logging::{ora_debug, ora_warn};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

use super::SESSION_SETUP_TIMEOUT;

const SESSION_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Owns every warm session and serializes work per chat surface.
pub(super) struct WarmSessions {
    pool: Mutex<WarmPool>,
    /// One gate per key so concurrent requests for the same surface queue
    /// instead of each starting a `session/new`. A client that mounts its chat
    /// view twice — React's development double-mount does exactly this — would
    /// otherwise leave an orphaned provider session behind on every open.
    gates: StdMutex<HashMap<WarmKey, Arc<Mutex<()>>>>,
    connections: ConnectionSupervisors,
    clock: SystemClock,
}

/// A warm session ready to be persisted as an Ora session.
pub(super) struct WarmAttachment {
    pub agent_cli: AgentCli,
    pub agent_session_id: String,
    pub cwd: PathBuf,
    pub available_commands: Vec<AvailableCommand>,
}

impl WarmSessions {
    pub(super) fn new(connections: ConnectionSupervisors, clock: SystemClock) -> Self {
        Self {
            pool: Mutex::new(WarmPool::default()),
            gates: StdMutex::new(HashMap::new()),
            connections,
            clock,
        }
    }

    /// Returns the warm session for one chat surface, creating it when needed.
    pub(super) async fn warm(
        &self,
        key: WarmKey,
        cwd: PathBuf,
    ) -> Result<(SessionId, Vec<SessionConfigOption>), BackendError> {
        let gate = self.gate(&key);
        let _guard = gate.lock().await;

        let supervisor = self.connections.for_agent(key.agent_cli);
        let connection = supervisor.current()?;
        let now = self.clock.now_timestamp_millis();
        let (decision, released) = {
            let mut pool = self.pool.lock().await;
            pool.lookup(&key, &cwd, connection.generation, now, || {
                UuidSessionIdGenerator::new().generate_session_id()
            })
        };
        self.release(released).await;

        match decision {
            WarmDecision::Ready {
                session_id,
                config_options,
                ..
            } => Ok((session_id, config_options)),
            WarmDecision::Create {
                session_id,
                cwd,
                replay,
            } => {
                let CreatedProvider {
                    agent_session_id,
                    config_options,
                    available_commands,
                } = self.create(key.agent_cli, &cwd).await?;
                let config_options = self
                    .replay(key.agent_cli, &agent_session_id, replay, config_options)
                    .await;
                let orphan = {
                    let mut pool = self.pool.lock().await;
                    pool.commit_created(
                        &session_id,
                        CreatedProvider {
                            agent_session_id,
                            config_options: config_options.clone(),
                            available_commands,
                        },
                        connection.generation,
                        self.clock.now_timestamp_millis(),
                    )
                };
                self.release(orphan).await;
                self.sweep().await;
                Ok((session_id, config_options))
            }
        }
    }

    /// Applies one configuration option to a warm session.
    ///
    /// A cold session records the choice without rebuilding: the client already
    /// renders the option list it was given, and the choice is replayed the next
    /// time a provider session is actually needed.
    pub(super) async fn set_config(
        &self,
        session_id: &SessionId,
        config_id: SessionConfigId,
        value: SessionConfigOptionValue,
    ) -> Option<Result<Vec<SessionConfigOption>, BackendError>> {
        self.refresh_generations().await;
        let now = self.clock.now_timestamp_millis();
        let target = self.pool.lock().await.config_target(session_id, now)?;
        let reported = match target {
            ConfigTarget::Deferred => None,
            ConfigTarget::Live {
                agent_cli,
                agent_session_id,
            } => match request_config_option(
                &self.connections,
                agent_cli,
                &agent_session_id,
                &config_id,
                &value,
            )
            .await
            {
                Ok(config_options) => Some(config_options),
                Err(error) => return Some(Err(error)),
            },
        };
        Some(Ok(self
            .pool
            .lock()
            .await
            .record_config(session_id, config_id, value, reported)))
    }

    /// Hands one warm session over for persistence against its owning Task.
    ///
    /// `cwd` is the Task's authoritative directory. A warm session created for a
    /// different one — the chat began before its Task existed, or a worktree
    /// moved — is rebuilt here rather than reused, because the alternative is an
    /// agent quietly working in the wrong directory.
    ///
    /// Rebuilding is transparent by design: the identifier the client holds keeps
    /// working, and a replay the agent rejects degrades to whatever the agent
    /// reports instead of failing the prompt the user already typed.
    pub(super) async fn take(
        &self,
        session_id: &SessionId,
        cwd: &Path,
    ) -> Result<WarmAttachment, BackendError> {
        self.refresh_generations().await;
        let Some(RebuildPlan {
            agent_cli,
            cwd: warm_cwd,
            replay,
        }) = self.pool.lock().await.rebuild_plan(session_id)
        else {
            return Err(runtime_internal(
                "warm_session_not_found",
                "warm session is no longer available",
            ));
        };
        if warm_cwd == cwd
            && let Some(attached) = self.pool.lock().await.take_for_attach(session_id)
        {
            return Ok(WarmAttachment {
                agent_cli: attached.agent_cli,
                agent_session_id: attached.agent_session_id,
                cwd: attached.cwd,
                available_commands: attached.available_commands,
            });
        }

        let created = self.create(agent_cli, cwd).await?;
        self.replay(
            agent_cli,
            &created.agent_session_id,
            replay,
            created.config_options,
        )
        .await;
        let superseded = self.pool.lock().await.forget(session_id);
        self.release(superseded).await;
        Ok(WarmAttachment {
            agent_cli,
            agent_session_id: created.agent_session_id,
            cwd: cwd.to_path_buf(),
            available_commands: created.available_commands,
        })
    }

    /// Drops provider sessions left behind by a CLI that restarted.
    ///
    /// A restart replaces the process, so every identifier from the previous
    /// generation is dead. Checking here rather than reacting to the supervisor
    /// keeps the pool free of callbacks, and the cost is one watch-channel read
    /// per CLI. The entries themselves survive as cold, so the identifiers
    /// clients already hold keep resolving.
    async fn refresh_generations(&self) {
        let mut pool = self.pool.lock().await;
        for agent_cli in AgentCli::ALL {
            if let Ok(connection) = self.connections.for_agent(agent_cli).current() {
                pool.invalidate_generation(agent_cli, connection.generation);
            }
        }
    }

    /// Retires idle and over-capacity sessions after the pool changed shape.
    async fn sweep(&self) {
        let released = self
            .pool
            .lock()
            .await
            .evict(self.clock.now_timestamp_millis());
        for session in released {
            self.release(Some(session)).await;
        }
    }

    /// Performs the `session/new` handshake for one warm session.
    ///
    /// The setup registration and the short-lived channel exist only to capture
    /// the command catalog: ACP announces it as an update immediately after the
    /// handshake, and a warm session has no consumer yet, so without them the
    /// announcement would be dropped and attaching could not report it. The
    /// channel is closed again right away — nothing streams into a warm session
    /// before it is attached.
    async fn create(
        &self,
        agent_cli: AgentCli,
        cwd: &Path,
    ) -> Result<CreatedProvider, BackendError> {
        let supervisor = self.connections.for_agent(agent_cli);
        let connection = supervisor.current()?;
        let _setup = supervisor.begin_session_setup();
        let response = timeout(
            SESSION_SETUP_TIMEOUT,
            connection.client.request::<_, NewSessionResponse>(
                AGENT_METHOD_NAMES.session_new,
                &NewSessionRequest::new(cwd),
            ),
        )
        .await
        .map_err(|_| {
            runtime_internal(
                "agent_start_timeout",
                "agent CLI session creation timed out",
            )
        })?
        .map_err(map_acp_error)?;
        ora_debug!(
            agent_cli = agent_cli.database_value(),
            agent_session_id = %response.session_id,
            "warm session created",
        );
        let mut channel = supervisor.open_session_channel(response.session_id.0.as_ref())?;
        let available_commands = collect_setup_commands(&mut channel).await;
        Ok(CreatedProvider {
            agent_session_id: response.session_id.to_string(),
            config_options: response.config_options.unwrap_or_default(),
            available_commands,
        })
    }

    /// Re-applies previously chosen options onto a freshly created session.
    ///
    /// Failures are deliberately swallowed: the user's selection may no longer
    /// exist for this directory or provider, and losing it is far better than
    /// refusing to start the conversation. The returned options describe what
    /// the agent actually has, so the client corrects itself.
    async fn replay(
        &self,
        agent_cli: AgentCli,
        agent_session_id: &str,
        replay: Vec<(SessionConfigId, SessionConfigOptionValue)>,
        mut config_options: Vec<SessionConfigOption>,
    ) -> Vec<SessionConfigOption> {
        for (config_id, value) in replay {
            match request_config_option(
                &self.connections,
                agent_cli,
                agent_session_id,
                &config_id,
                &value,
            )
            .await
            {
                Ok(updated) => config_options = updated,
                Err(error) => ora_warn!(
                    agent_cli = agent_cli.database_value(),
                    config_id = %config_id,
                    error = %error,
                    "warm session configuration replay failed",
                ),
            }
        }
        config_options
    }

    /// Removes a provider session Ora created but never handed to the user.
    ///
    /// Deleting is safe only because these sessions were never exposed: they
    /// carry no history and no Ora record. Sessions the user can see are never
    /// deleted from the provider, only closed.
    async fn release(&self, released: Option<ReleasedSession>) {
        let Some(released) = released else {
            return;
        };
        let Ok(connection) = self.connections.for_agent(released.agent_cli).current() else {
            return;
        };
        if connection.generation != released.generation {
            return;
        }
        if connection.delete_session_supported {
            let _ = timeout(
                SESSION_RELEASE_TIMEOUT,
                connection.client.request::<_, DeleteSessionResponse>(
                    AGENT_METHOD_NAMES.session_delete,
                    &DeleteSessionRequest::new(released.agent_session_id.clone()),
                ),
            )
            .await;
        } else if connection.close_session_supported {
            let _ = timeout(
                SESSION_RELEASE_TIMEOUT,
                connection.client.request::<_, CloseSessionResponse>(
                    AGENT_METHOD_NAMES.session_close,
                    &CloseSessionRequest::new(released.agent_session_id.clone()),
                ),
            )
            .await;
        }
    }

    /// Returns the per-key gate, creating it on first use.
    fn gate(&self, key: &WarmKey) -> Arc<Mutex<()>> {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        gates.entry(key.clone()).or_default().clone()
    }
}

/// Sends one `session/set_config_option` request and returns the agent's report.
///
/// Shared with persisted sessions: option changes are addressed by provider
/// session id, so they do not need to travel through a session's serialized
/// actor and cannot be blocked by a prompt already streaming there.
pub(super) async fn request_config_option(
    connections: &ConnectionSupervisors,
    agent_cli: AgentCli,
    agent_session_id: &str,
    config_id: &SessionConfigId,
    value: &SessionConfigOptionValue,
) -> Result<Vec<SessionConfigOption>, BackendError> {
    let connection = connections.for_agent(agent_cli).current()?;
    let response = timeout(
        SESSION_SETUP_TIMEOUT,
        connection
            .client
            .request::<_, SetSessionConfigOptionResponse>(
                AGENT_METHOD_NAMES.session_set_config_option,
                &SetSessionConfigOptionRequest::new(
                    agent_session_id.to_string(),
                    config_id.clone(),
                    value.clone(),
                ),
            ),
    )
    .await
    .map_err(|_| {
        runtime_internal(
            "agent_config_timeout",
            "agent CLI configuration update timed out",
        )
    })?
    .map_err(map_acp_error)?;
    Ok(response.config_options)
}

//! Drives warm provider sessions between the pool's decisions and ACP.
//!
//! [`WarmPool`] decides what should happen; this module performs it. Nothing
//! here holds the runtime's lifecycle lock, so opening a chat surface never
//! serializes against prompts running in other sessions.

use super::collect_setup_commands;
use super::connection::ConnectionSupervisors;
use super::support::{map_acp_error, runtime_internal};
use super::warm_pool::{
    ClaimDecision, ConfigTarget, CreatePlan, CreatedProvider, Install, RebuildPlan,
    ReleasedSession, Reservation, WarmDecision, WarmKey, WarmPool,
};
use crate::BackendError;
use crate::clock::SystemClock;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::AvailableCommand;
use agent_client_protocol_schema::v1::{
    CloseSessionRequest, CloseSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    NewSessionRequest, NewSessionResponse,
};
use agent_client_protocol_schema::v1::{
    SessionConfigId, SessionConfigOption, SessionConfigOptionValue, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse,
};
use ora_application::{Clock, SessionIdGenerator, UuidSessionIdGenerator};
use ora_contracts::WarmSessionTarget;
use ora_domain::{AgentCli, SessionId};
use ora_logging::{ora_debug, ora_warn};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

use super::SESSION_SETUP_TIMEOUT;

const SESSION_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Owns every warm session and serializes work per chat surface.
pub(super) struct WarmSessions {
    /// Guarded by a blocking mutex on purpose. [`WarmPool`] performs no I/O, so
    /// no critical section here spans an `.await`, and a reservation can only be
    /// returned from `Drop` — which cannot await — if taking this lock does not
    /// either. Every future that reaches this module must be `Send`, so the
    /// compiler rejects any later change that holds the guard across a
    /// suspension point rather than letting it deadlock the executor.
    pool: StdMutex<WarmPool>,
    /// One gate per key so concurrent requests for the same surface queue
    /// instead of each starting a `session/new`. A client that mounts its chat
    /// view twice — React's development double-mount does exactly this — would
    /// otherwise leave an orphaned provider session behind on every open.
    gates: StdMutex<HashMap<WarmKey, Arc<Mutex<()>>>>,
    connections: ConnectionSupervisors,
    clock: SystemClock,
}

/// A warm session ready to be bound to an Ora session.
pub(super) struct WarmAttachment {
    pub agent_cli: AgentCli,
    pub agent_session_id: String,
    pub cwd: PathBuf,
    pub available_commands: Vec<AvailableCommand>,
    /// What this provider session currently offers to configure.
    ///
    /// Needed by a caller that rebinds an existing Ora session: ACP reports
    /// configuration only while creating or loading a session, so the claim is
    /// the last point at which the incoming agent's model list can be learned.
    pub config_options: Vec<SessionConfigOption>,
}

/// A warm session held for one attach attempt, returned to the pool on drop.
///
/// The reservation lives in a value rather than in a pair of calls because the
/// caller cannot be relied upon to make the second one. Attaching waits on the
/// runtime's lifecycle lock while holding a reservation, and the Desktop command surface
/// drops the whole request future when its client disconnects; unwinding from a
/// panic loses the call the same way. Both paths run `Drop`, which is the only
/// reason a lost caller cannot strand an entry — eviction deliberately skips
/// reserved entries, so a reservation nothing releases would pin a provider
/// session that no bound can ever reclaim.
pub(super) struct WarmReservation<'a> {
    /// Borrowed rather than reached through [`WarmSessions`] so the reservation
    /// depends only on the state it actually changes, and can be exercised
    /// without standing up a CLI connection.
    pool: &'a StdMutex<WarmPool>,
    session_id: SessionId,
    attachment: WarmAttachment,
    /// Set by `commit`, which already removed the entry; `Drop` then has
    /// nothing to return.
    committed: bool,
}

impl<'a> WarmReservation<'a> {
    fn new(
        pool: &'a StdMutex<WarmPool>,
        session_id: SessionId,
        attachment: WarmAttachment,
    ) -> Self {
        Self {
            pool,
            session_id,
            attachment,
            committed: false,
        }
    }

    /// Describes the provider session this reservation offers the caller.
    pub(super) fn attachment(&self) -> &WarmAttachment {
        &self.attachment
    }

    /// Finalizes the handoff once the caller has actually persisted the
    /// session: the provider session now belongs to it, not the warm pool.
    pub(super) fn commit(mut self) {
        lock_pool(self.pool).commit_attach(&self.session_id);
        self.committed = true;
    }
}

impl Drop for WarmReservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            lock_pool(self.pool).release_reservation(&self.session_id);
        }
    }
}

/// Reports that another attach already owns the warm session being claimed.
///
/// Failing is the point: the alternative — rebuilding — would hand this caller
/// a provider session while releasing the one the first attach is persisting,
/// leaving a session the user can see but the agent has already dropped.
fn attach_in_flight() -> BackendError {
    runtime_internal(
        "warm_session_attach_in_flight",
        "warm session is already being attached",
    )
}

/// Locks a warm pool, adopting the state a panicking caller left behind.
///
/// Nothing here can be left half-written: every pool method replaces whole
/// entries, so a poisoned lock carries no torn invariant worth propagating, and
/// refusing to serve chat surfaces for the rest of the process would be the
/// larger failure.
fn lock_pool(pool: &StdMutex<WarmPool>) -> MutexGuard<'_, WarmPool> {
    pool.lock().unwrap_or_else(PoisonError::into_inner)
}

impl WarmSessions {
    pub(super) fn new(connections: ConnectionSupervisors, clock: SystemClock) -> Self {
        Self {
            pool: StdMutex::new(WarmPool::default()),
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
        let (decision, released) =
            lock_pool(&self.pool).lookup(&key, &cwd, connection.generation, now, || {
                UuidSessionIdGenerator::new().generate_session_id()
            });
        self.release(released).await;

        match decision {
            WarmDecision::Ready {
                session_id,
                config_options,
                ..
            } => Ok((session_id, config_options)),
            WarmDecision::Create(CreatePlan {
                session_id,
                cwd,
                replay,
            }) => {
                let CreatedProvider {
                    agent_session_id,
                    config_options,
                    available_commands,
                } = self.create(key.agent_cli, &session_id, &cwd).await?;
                let config_options = self
                    .replay(key.agent_cli, &agent_session_id, replay, config_options)
                    .await;
                let installed = lock_pool(&self.pool).commit_created(
                    &session_id,
                    CreatedProvider {
                        agent_session_id: agent_session_id.clone(),
                        config_options: config_options.clone(),
                        available_commands,
                    },
                    connection.generation,
                    self.clock.now_timestamp_millis(),
                );
                let orphan = match installed {
                    Install::Accepted(superseded) => superseded,
                    Install::Refused => Some(ReleasedSession {
                        agent_cli: key.agent_cli,
                        agent_session_id,
                        generation: connection.generation,
                    }),
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
        let target = lock_pool(&self.pool).config_target(session_id, now)?;
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
        Some(Ok(
            lock_pool(&self.pool).record_config(session_id, config_id, value, reported)
        ))
    }

    /// Reserves one warm session for persistence against its owning Task,
    /// without removing it from the pool yet.
    ///
    /// `cwd` is the Task's authoritative directory. A warm session created for a
    /// different one — the chat began before its Task existed, or a worktree
    /// moved — is rebuilt here rather than reused, because the alternative is an
    /// agent quietly working in the wrong directory.
    ///
    /// Rebuilding is transparent by design: the identifier the client holds keeps
    /// working, and a replay the agent rejects degrades to whatever the agent
    /// reports instead of failing the prompt the user already typed.
    ///
    /// The returned [`WarmReservation`] finishes the handoff: `commit` once the
    /// caller's own persistence steps succeed, and otherwise nothing at all —
    /// dropping it returns the entry to the pool. Attaching can still fail after
    /// this point (a channel or repository error), and it can also be abandoned
    /// without failing, so tying the release to a value rather than to a call
    /// keeps either outcome from stranding the client's id or pinning the
    /// provider session reserved here.
    pub(super) async fn take(
        &self,
        session_id: &SessionId,
        cwd: &Path,
    ) -> Result<WarmReservation<'_>, BackendError> {
        self.refresh_generations().await;
        let Some(RebuildPlan {
            agent_cli, replay, ..
        }) = lock_pool(&self.pool).rebuild_plan(session_id)
        else {
            return Err(runtime_internal(
                "warm_session_not_found",
                "warm session is no longer available",
            ));
        };
        // Bound to a `let` rather than matched inline: a match holds its
        // scrutinee's temporaries for every arm, and the pool lock is not
        // reentrant against the `Drop` the arms below build on.
        let reservation = lock_pool(&self.pool).reserve_for_attach(session_id, cwd);
        match reservation {
            Reservation::Held(attached) => {
                return Ok(WarmReservation::new(
                    &self.pool,
                    session_id.clone(),
                    WarmAttachment {
                        agent_cli: attached.agent_cli,
                        agent_session_id: attached.agent_session_id,
                        cwd: attached.cwd,
                        available_commands: attached.available_commands,
                        config_options: attached.config_options,
                    },
                ));
            }
            Reservation::Unavailable => return Err(attach_in_flight()),
            Reservation::NeedsRebuild => {}
        }

        let connection = self.connections.for_agent(agent_cli).current()?;
        let created = self.create(agent_cli, session_id, cwd).await?;
        let config_options = self
            .replay(
                agent_cli,
                &created.agent_session_id,
                replay,
                created.config_options,
            )
            .await;
        let installed = lock_pool(&self.pool).replace_and_reserve(
            session_id,
            cwd.to_path_buf(),
            CreatedProvider {
                agent_session_id: created.agent_session_id.clone(),
                config_options: config_options.clone(),
                available_commands: created.available_commands.clone(),
            },
            connection.generation,
            self.clock.now_timestamp_millis(),
        );
        let Install::Accepted(superseded) = installed else {
            // Another attach took the entry during the handshake above, so the
            // session just created belongs to nobody.
            self.release(Some(ReleasedSession {
                agent_cli,
                agent_session_id: created.agent_session_id,
                generation: connection.generation,
            }))
            .await;
            return Err(attach_in_flight());
        };
        // Built before the release below, which is the last `.await` a caller
        // can be dropped at while this entry is already reserved.
        let reservation = WarmReservation::new(
            &self.pool,
            session_id.clone(),
            WarmAttachment {
                agent_cli,
                agent_session_id: created.agent_session_id,
                cwd: cwd.to_path_buf(),
                available_commands: created.available_commands,
                config_options,
            },
        );
        self.release(superseded).await;
        Ok(reservation)
    }

    /// Claims the warm session backing one chat surface, addressed by its key.
    ///
    /// This is [`WarmSessions::take`] for a caller that has no warm identifier to
    /// name. Rebinding an existing conversation onto another CLI wants the
    /// session this client already warmed while its picker was showing that
    /// CLI's models — including any model chosen on it — but the identifier of
    /// that session is the pool's own, never handed back by the client. Naming it
    /// by key instead is what lets the picker and the rebind meet.
    ///
    /// Resolving and reserving happen in one critical section, so no other claim
    /// can take the session in between. And because resolution skips reserved
    /// entries, a key whose session is being attached, was evicted, or was
    /// already consumed resolves to a fresh entry that is handshaken here: the
    /// caller never fails merely because what it warmed earlier is gone.
    pub(super) async fn claim(
        &self,
        key: WarmKey,
        cwd: &Path,
    ) -> Result<WarmReservation<'_>, BackendError> {
        let gate = self.gate(&key);
        let _guard = gate.lock().await;

        let connection = self.connections.for_agent(key.agent_cli).current()?;
        let now = self.clock.now_timestamp_millis();
        let (decision, released) =
            lock_pool(&self.pool).lookup_and_reserve(&key, cwd, connection.generation, now, || {
                UuidSessionIdGenerator::new().generate_session_id()
            });
        self.release(released).await;

        let CreatePlan {
            session_id,
            cwd,
            replay,
        } = match decision {
            ClaimDecision::Held(attached) => {
                return Ok(WarmReservation::new(
                    &self.pool,
                    attached.session_id,
                    WarmAttachment {
                        agent_cli: attached.agent_cli,
                        agent_session_id: attached.agent_session_id,
                        cwd: attached.cwd,
                        available_commands: attached.available_commands,
                        config_options: attached.config_options,
                    },
                ));
            }
            ClaimDecision::Create(plan) => plan,
        };

        let created = self.create(key.agent_cli, &session_id, &cwd).await?;
        let config_options = self
            .replay(
                key.agent_cli,
                &created.agent_session_id,
                replay,
                created.config_options,
            )
            .await;
        let installed = lock_pool(&self.pool).replace_and_reserve(
            &session_id,
            cwd.clone(),
            CreatedProvider {
                agent_session_id: created.agent_session_id.clone(),
                config_options: config_options.clone(),
                available_commands: created.available_commands.clone(),
            },
            connection.generation,
            self.clock.now_timestamp_millis(),
        );
        let Install::Accepted(superseded) = installed else {
            // An attach reserved the entry during the handshake above, so the
            // session just created belongs to nobody.
            self.release(Some(ReleasedSession {
                agent_cli: key.agent_cli,
                agent_session_id: created.agent_session_id,
                generation: connection.generation,
            }))
            .await;
            return Err(attach_in_flight());
        };
        // Built before the releases below, which are the last `.await`s a caller
        // can be dropped at while this entry is already reserved.
        let reservation = WarmReservation::new(
            &self.pool,
            session_id,
            WarmAttachment {
                agent_cli: key.agent_cli,
                agent_session_id: created.agent_session_id,
                cwd,
                available_commands: created.available_commands,
                config_options,
            },
        );
        self.release(superseded).await;
        self.sweep().await;
        Ok(reservation)
    }

    /// Drops provider sessions left behind by a CLI that restarted.
    ///
    /// A restart replaces the process, so every identifier from the previous
    /// generation is dead. Checking here rather than reacting to the supervisor
    /// keeps the pool free of callbacks, and the cost is one watch-channel read
    /// per CLI. The entries themselves survive as cold, so the identifiers
    /// clients already hold keep resolving.
    async fn refresh_generations(&self) {
        let mut pool = lock_pool(&self.pool);
        for agent_cli in AgentCli::ALL {
            if let Ok(connection) = self.connections.for_agent(agent_cli).current() {
                pool.invalidate_generation(agent_cli, connection.generation);
            }
        }
    }

    /// Releases every warm session whose chat surface was deleted.
    ///
    /// Nothing else reclaims these: their targets are gone, so no request will
    /// ever name them again and no bound will reach them in time to matter.
    pub(super) async fn discard(&self, targets: &[WarmSessionTarget]) {
        let released = lock_pool(&self.pool).discard_targets(targets);
        for session in released {
            self.release(Some(session)).await;
        }
    }

    /// Retires over-capacity sessions after the pool changed shape.
    async fn sweep(&self) {
        let released = lock_pool(&self.pool).evict();
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
        ora_session_id: &SessionId,
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
        let mut channel = supervisor
            .open_session_channel(response.session_id.0.as_ref(), ora_session_id.as_ref())?;
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

#[cfg(test)]
mod tests {
    use super::{WarmAttachment, WarmPool, WarmReservation, lock_pool};
    use crate::agent_runtime::WarmOwner;
    use crate::agent_runtime::warm_pool::{AttachedWarm, CreatedProvider, Reservation, WarmKey};
    use ora_contracts::WarmSessionTarget;
    use ora_domain::{AgentCli, SessionId};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex as StdMutex;

    const GENERATION: u64 = 1;

    /// Builds a pool holding one live warm session already reserved for attach.
    fn reserved_pool() -> (StdMutex<WarmPool>, SessionId) {
        let mut pool = WarmPool::default();
        let key = WarmKey {
            target: WarmSessionTarget::Task {
                task_id: "task-1".to_string(),
            },
            agent_cli: AgentCli::OpenCode,
            owner: WarmOwner::Interactive,
        };
        let session_id = SessionId::new("session-1");
        let _ = pool.lookup(&key, Path::new("/repo"), GENERATION, 0, || {
            session_id.clone()
        });
        let _ = pool.commit_created(
            &session_id,
            CreatedProvider {
                agent_session_id: "agent-session-1".to_string(),
                config_options: Vec::new(),
                available_commands: Vec::new(),
            },
            GENERATION,
            0,
        );
        let _ = pool.reserve_for_attach(&session_id, Path::new("/repo"));
        (StdMutex::new(pool), session_id)
    }

    fn attachment() -> WarmAttachment {
        WarmAttachment {
            agent_cli: AgentCli::OpenCode,
            agent_session_id: "agent-session-1".to_string(),
            cwd: PathBuf::from("/repo"),
            available_commands: Vec::new(),
            config_options: Vec::new(),
        }
    }

    /// Verifies dropping a reservation returns the entry, which is what an
    /// attach lost to a panic or a disconnected client relies on to avoid
    /// pinning a provider session no bound would reclaim.
    #[test]
    fn returns_the_entry_when_the_reservation_is_dropped() {
        let (pool, session_id) = reserved_pool();

        drop(WarmReservation::new(
            &pool,
            session_id.clone(),
            attachment(),
        ));

        assert_eq!(
            lock_pool(&pool).reserve_for_attach(&session_id, Path::new("/repo")),
            Reservation::Held(AttachedWarm {
                session_id,
                agent_cli: AgentCli::OpenCode,
                agent_session_id: "agent-session-1".to_string(),
                cwd: PathBuf::from("/repo"),
                available_commands: vec![],
                config_options: vec![],
            })
        );
    }

    /// Verifies a committed reservation removes the entry instead of returning
    /// it, so the provider session belongs to the persisted session alone.
    #[test]
    fn removes_the_entry_when_the_reservation_is_committed() {
        let (pool, session_id) = reserved_pool();

        WarmReservation::new(&pool, session_id.clone(), attachment()).commit();

        assert_eq!(lock_pool(&pool).rebuild_plan(&session_id), None);
    }
}

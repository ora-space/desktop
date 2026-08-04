mod actor;
mod connection;
mod history;
mod models;
mod routing;
mod stream;
mod support;

use history::{RecordOutcome, SessionRecorder};
pub use stream::SessionEventStream;
use support::*;

use crate::clock::SystemClock;
use crate::task::resolve_task_cwd;
use crate::{BackendError, ErrorClassification};
use connection::{ConnectionSupervisor, ConnectionSupervisors};
use ora_application::{Clock, SessionIdGenerator, SessionRepository, UuidSessionIdGenerator};
use ora_contracts::acp::content::ContentBlock;
use ora_contracts::acp::session::SessionUpdate;
use ora_contracts::acp::slash_command::AvailableCommand;
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionResponse, LoadSessionEvent,
    LoadSessionRequest, PromptSessionEvent, PromptSessionRequest, RespondToPermissionRequest,
    RespondToPermissionResponse, ResumeSessionHistoryRequest, ResumeSessionHistoryResponse,
    StopSessionRequest, StopSessionResponse, SwitchSessionAgentRequest, SwitchSessionAgentResponse,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{AgentCli, AuditFields, HistoryState, Session, SessionId, SessionStatus, TaskId};
use ora_history::{binding_needs_handoff, read_session_history};
use ora_logging::{ora_debug, ora_warn};
use routing::SessionChannel;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
const SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const CANCELLATION_GRACE: Duration = Duration::from_secs(5);
const CONTRACT_QUEUE_CAPACITY: usize = 256;
const MAX_PROMPT_BYTES: usize = 16 * 1024 * 1024;

/// Coordinates one serialized actor per Ora session on its selected supervised CLI connection.
#[derive(Clone)]
pub(crate) struct AgentRuntimeManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    pool: RepositoryPool,
    actors: RwLock<HashMap<SessionId, RuntimeActorHandle>>,
    lifecycle: tokio::sync::Mutex<()>,
    next_operation_id: AtomicU64,
    connections: ConnectionSupervisors,
    home_directory: PathBuf,
    sessions_root: PathBuf,
    clock: SystemClock,
}

#[derive(Clone)]
struct RuntimeActorHandle {
    commands: mpsc::UnboundedSender<RuntimeCommand>,
}

pub(super) enum RuntimeCommand {
    Load {
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
        accepted: oneshot::Sender<Result<(), BackendError>>,
    },
    Prompt {
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        events: mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
        accepted: oneshot::Sender<Result<(), BackendError>>,
    },
    RespondToPermission {
        request: RespondToPermissionRequest,
        response: oneshot::Sender<Result<RespondToPermissionResponse, BackendError>>,
    },
    Stop {
        response: oneshot::Sender<Result<StopSessionResponse, BackendError>>,
    },
    Cancel {
        operation_id: u64,
    },
}

struct RuntimeActor {
    session: Session,
    cwd: PathBuf,
    repository: SqliteSessionRepository,
    clock: SystemClock,
    connection: ConnectionSupervisor,
    channel: Option<SessionChannel>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    recorder: SessionRecorder,
    sessions_root: PathBuf,
    /// Whether the current provider binding still has to be told the history.
    ///
    /// Switching agents rebinds eagerly but injects lazily, so this is answered
    /// from the record when the actor opens and cleared once a prompt carries the
    /// transcript across.
    handoff_pending: bool,
}

/// One freshly created provider session and everything needed to route it.
struct ProviderSession {
    agent_session_id: String,
    channel: SessionChannel,
    available_commands: Vec<AvailableCommand>,
    supervisor: ConnectionSupervisor,
}

/// One session's opened recorder together with what reading its file revealed.
struct OpenedRecorder {
    recorder: SessionRecorder,
    handoff_pending: bool,
    /// Set when the history could not be read, which degrades the session.
    ///
    /// A history Ora cannot read is one it cannot safely extend: appending
    /// without knowing the positions already used would overwrite them.
    failure: Option<String>,
}

impl AgentRuntimeManager {
    /// Builds the manager, reconciles stale rows, and immediately starts the shared supervisor.
    pub(crate) fn new(
        pool: RepositoryPool,
        home_directory: PathBuf,
        sessions_root: PathBuf,
        clock: SystemClock,
    ) -> Result<Self, BackendError> {
        reconcile_running_sessions(&pool, clock)?;
        let connections = ConnectionSupervisors::start(pool.clone(), home_directory.clone(), clock);
        Ok(Self {
            inner: Arc::new(ManagerInner {
                pool,
                actors: RwLock::new(HashMap::new()),
                lifecycle: tokio::sync::Mutex::new(()),
                next_operation_id: AtomicU64::new(1),
                connections,
                home_directory,
                sessions_root,
                clock,
            }),
        })
    }

    /// Lists model identifiers from every CLI whose discovery command succeeds.
    pub(crate) async fn list_agent_models(&self) -> ora_contracts::ListAgentModelsResponse {
        models::list_agent_models(&self.inner.home_directory).await
    }

    /// Creates a session over the selected application-scoped CLI connection.
    pub(crate) async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let agent_cli = domain_agent_cli(request.agent_cli);
        let cwd = resolve_task_cwd(&self.inner.pool, &TaskId::new(request.task_id.clone()))?;
        let provider = self.open_provider_session(agent_cli, &cwd).await?;
        let now = self.inner.clock.now_timestamp_millis();
        let session = Session::new(
            UuidSessionIdGenerator::new().generate_session_id(),
            TaskId::new(request.task_id),
            agent_cli,
            provider.agent_session_id,
            SessionStatus::Running,
            AuditFields::new(now, now, false),
        );
        SqliteSessionRepository::new(self.inner.pool.clone())
            .create_session(session.clone())
            .map_err(|source| {
                BackendError::internal("failed to persist agent CLI session", source)
            })?;
        ora_debug!(
            session_id = %session.id,
            agent_session_id = %session.agent_session_id,
            "session created",
        );
        // The header opens the file this conversation owns for the rest of its
        // life, so it is written before the session can be prompted.
        let mut opened = self.open_recorder(&session)?;
        let outcome = match opened.failure.take() {
            Some(reason) => RecordOutcome::JustFailed { reason },
            None => opened.recorder.record_meta(&session, &cwd),
        };
        let session = self.settle_record(session, outcome);
        self.insert_actor(
            session.clone(),
            cwd,
            provider.supervisor,
            Some(provider.channel),
            opened.recorder,
            /*handoff_pending*/ false,
        )?;
        Ok(CreateSessionResponse {
            session: contract_session(session),
            available_commands: provider.available_commands,
        })
    }

    /// Moves one existing conversation onto a different agent CLI.
    ///
    /// The new provider session is created before anything is torn down, so a CLI
    /// that is unavailable or slow to hand shake leaves the conversation exactly
    /// where it was. Only the binding changes: the identifier, the task, and the
    /// recorded history all continue.
    pub(crate) async fn switch_agent(
        &self,
        request: SwitchSessionAgentRequest,
    ) -> Result<SwitchSessionAgentResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let target = domain_agent_cli(request.agent_cli);
        if target == session.agent_cli {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::SessionAgentUnchanged(EmptyErrorParams {}),
                "session already runs on this agent CLI",
            ));
        }
        if let HistoryState::Degraded { .. } = session.history_state {
            return Err(history_degraded());
        }
        let cwd = resolve_task_cwd(&self.inner.pool, &session.task_id)?;
        let provider = self.open_provider_session(target, &cwd).await?;

        // Only now is the move certain, so the old binding can be released. Its
        // context is not reusable afterwards: work done on the new agent would be
        // missing from it, and switching back re-injects the transcript instead.
        let previous = session.agent_cli;
        let (session, recorder) = match self
            .rebind_to_provider(&session.id, previous, target, &provider)
            .await
        {
            Ok(rebound) => rebound,
            Err(error) => {
                // `session/new` already succeeded, so the CLI is holding a session
                // Ora has just decided not to use. Nothing else will ever close it:
                // dropping the channel unregisters routing only, and no Ora row
                // names this provider session for a later attempt to find.
                close_provider_session(&provider).await;
                return Err(error);
            }
        };
        self.insert_actor(
            session.clone(),
            cwd,
            provider.supervisor,
            Some(provider.channel),
            recorder,
            // The new agent knows nothing; the next prompt carries the transcript.
            /*handoff_pending*/
            true,
        )?;
        Ok(SwitchSessionAgentResponse {
            session: contract_session(session),
            available_commands: provider.available_commands,
        })
    }

    /// Moves one stored session onto a provider binding that already exists.
    ///
    /// Separate from `switch_agent` because every step here can fail *after*
    /// `session/new` succeeded, and each of those failures owes the provider a
    /// `session/close`. Keeping them in one fallible region gives the caller a
    /// single place to release the binding instead of a release per `?`.
    async fn rebind_to_provider(
        &self,
        session_id: &SessionId,
        previous: AgentCli,
        target: AgentCli,
        provider: &ProviderSession,
    ) -> Result<(Session, SessionRecorder), BackendError> {
        if let Some(handle) = self.lookup_actor(session_id)? {
            self.stop_actor(handle).await?;
        }
        self.actors_write()?.remove(session_id);

        let now = self.inner.clock.now_timestamp_millis();
        let session = self
            .find_session(session_id.as_ref())?
            .with_binding(target, provider.agent_session_id.clone(), now)
            .with_status(SessionStatus::Running, now);
        SqliteSessionRepository::new(self.inner.pool.clone())
            .update_session(session.clone())
            .map_err(|source| BackendError::internal("failed to rebind agent session", source))?;
        ora_debug!(
            session_id = %session.id,
            from = previous.database_value(),
            to = target.database_value(),
            "session agent switched",
        );

        let mut opened = self.open_recorder(&session)?;
        let outcome = match opened.failure.take() {
            Some(reason) => RecordOutcome::JustFailed { reason },
            None => opened.recorder.record_agent_switch(
                previous,
                target,
                provider.agent_session_id.clone(),
            ),
        };
        Ok((self.settle_record(session, outcome), opened.recorder))
    }

    /// Returns a session whose history writes failed to a writable state.
    ///
    /// The gap is recorded before anything else, so what the failure cost stays
    /// visible to everyone who reads the file afterwards — including the agent
    /// that receives this conversation next.
    pub(crate) async fn resume_history(
        &self,
        request: ResumeSessionHistoryRequest,
    ) -> Result<ResumeSessionHistoryResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let HistoryState::Degraded { reason } = session.history_state.clone() else {
            return Ok(ResumeSessionHistoryResponse {
                session: contract_session(session),
            });
        };
        // The live actor still holds a stopped recorder, so it is discarded and
        // rebuilt from the recovered row on the session's next operation.
        if let Some(handle) = self.lookup_actor(&session.id)? {
            self.stop_actor(handle).await?;
        }
        self.actors_write()?.remove(&session.id);

        let mut opened = self.open_recorder(&session)?;
        if let Some(failure) = opened.failure {
            return Err(BackendError::new(
                ErrorClassification::Internal,
                PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
                format!("session history is still unreadable: {failure}"),
            ));
        }
        if let RecordOutcome::JustFailed { reason } = opened.recorder.resume(reason) {
            return Err(BackendError::new(
                ErrorClassification::Internal,
                PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
                format!("session history is still unwritable: {reason}"),
            ));
        }
        let now = self.inner.clock.now_timestamp_millis();
        let session = self
            .find_session(&request.session_id)?
            .with_history_state(HistoryState::Writable, now);
        SqliteSessionRepository::new(self.inner.pool.clone())
            .update_session(session.clone())
            .map_err(|source| BackendError::internal("failed to resume session history", source))?;
        Ok(ResumeSessionHistoryResponse {
            session: contract_session(session),
        })
    }

    /// Runs the provider handshake for one CLI and routes the resulting session.
    async fn open_provider_session(
        &self,
        agent_cli: AgentCli,
        cwd: &std::path::Path,
    ) -> Result<ProviderSession, BackendError> {
        use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
        use ora_contracts::acp::session::{NewSessionRequest, NewSessionResponse};
        use tokio::time::timeout;

        let supervisor = self.inner.connections.for_agent(agent_cli);
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
        .map_err(|source| BackendError::internal("agent CLI session creation timed out", source))?
        .map_err(map_acp_error)?;
        let mut channel = supervisor.open_session_channel(response.session_id.0.as_ref())?;
        let available_commands = collect_setup_commands(&mut channel).await;
        Ok(ProviderSession {
            agent_session_id: response.session_id.to_string(),
            channel,
            available_commands,
            supervisor,
        })
    }

    /// Opens one session's recorder, resuming its position counter from the file.
    fn open_recorder(&self, session: &Session) -> Result<OpenedRecorder, BackendError> {
        let root = &self.inner.sessions_root;
        let session_id = session.id.as_ref();
        match read_session_history(root, session_id) {
            Ok(history) => {
                if history.dropped_lines > 0 {
                    ora_warn!(
                        session_id = %session.id,
                        dropped_lines = history.dropped_lines,
                        "session history contains unreadable lines",
                    );
                }
                let recorder = SessionRecorder::open(
                    root,
                    session_id,
                    history.next_seq,
                    &session.history_state,
                )
                .map_err(|source| {
                    BackendError::internal("failed to open session history", source)
                })?;
                Ok(OpenedRecorder {
                    recorder,
                    handoff_pending: binding_needs_handoff(&history),
                    failure: None,
                })
            }
            Err(error) => {
                // Appending without knowing which positions are already used would
                // overwrite them, so an unreadable file stops recording outright.
                ora_warn!(session_id = %session.id, error = %error, "session history is unreadable");
                let failure = error.to_string();
                let recorder = SessionRecorder::open(
                    root,
                    session_id,
                    0,
                    &HistoryState::Degraded {
                        reason: failure.clone(),
                    },
                )
                .map_err(|source| {
                    BackendError::internal("failed to open session history", source)
                })?;
                Ok(OpenedRecorder {
                    recorder,
                    handoff_pending: false,
                    failure: Some(failure),
                })
            }
        }
    }

    /// Persists the degraded state when a recording attempt just broke the history.
    fn settle_record(&self, session: Session, outcome: RecordOutcome) -> Session {
        let RecordOutcome::JustFailed { reason } = outcome else {
            return session;
        };
        let now = self.inner.clock.now_timestamp_millis();
        let degraded = session.with_history_state(HistoryState::Degraded { reason }, now);
        match SqliteSessionRepository::new(self.inner.pool.clone()).update_session(degraded.clone())
        {
            Ok(stored) => stored,
            Err(error) => {
                ora_warn!(error = %error, "failed to persist degraded session history state");
                degraded
            }
        }
    }

    /// Starts an explicit ACP load stream for one persisted Ora session.
    pub(crate) async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> Result<SessionEventStream<LoadSessionEvent>, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let handle = self.actor_for(session)?;
        let operation_id = self.inner.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let (events_sender, events) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
        let (accepted_sender, accepted) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Load {
                operation_id,
                events: events_sender,
                accepted: accepted_sender,
            })
            .map_err(runtime_unavailable_with)?;
        accepted.await.map_err(runtime_unavailable_with)??;
        Ok(SessionEventStream::new(
            events,
            handle.commands,
            operation_id,
        ))
    }

    /// Starts one structured ACP prompt stream after validating the public payload limit.
    pub(crate) async fn prompt_session(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        let prompt = request.prompt;
        if prompt.is_empty()
            || prompt.iter().all(|content| {
                matches!(content, ContentBlock::Text(text) if text.text.trim().is_empty())
            })
        {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PromptEmpty(EmptyErrorParams {}),
                "prompt must contain text or media",
            ));
        }
        let prompt_bytes = serde_json::to_vec(&prompt)
            .map_err(|_| runtime_internal("prompt_encoding_failed", "failed to encode prompt"))?
            .len();
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::PromptTooLarge(EmptyErrorParams {}),
                "prompt exceeds 16 MiB",
            ));
        }
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        if session.status != SessionStatus::Running {
            return Err(session_stopped());
        }
        // A session whose history stopped recording refuses new turns rather than
        // producing conversation that would never be part of the record.
        if let HistoryState::Degraded { .. } = session.history_state {
            return Err(history_degraded());
        }
        let handle = self.actor_for(session)?;
        let operation_id = self.inner.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let (events_sender, events) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
        let (accepted_sender, accepted) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Prompt {
                operation_id,
                prompt,
                events: events_sender,
                accepted: accepted_sender,
            })
            .map_err(runtime_unavailable_with)?;
        accepted.await.map_err(runtime_unavailable_with)??;
        Ok(SessionEventStream::new(
            events,
            handle.commands,
            operation_id,
        ))
    }

    /// Routes one opaque permission response to the actor that registered the request.
    pub(crate) async fn respond_to_permission(
        &self,
        request: RespondToPermissionRequest,
    ) -> Result<RespondToPermissionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let handle = self.actor_for(session)?;
        let (response_sender, response) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::RespondToPermission {
                request,
                response: response_sender,
            })
            .map_err(runtime_unavailable_with)?;
        response.await.map_err(runtime_unavailable_with)?
    }

    /// Stops one logical session without terminating its shared CLI process.
    pub(crate) async fn stop_session(
        &self,
        request: StopSessionRequest,
    ) -> Result<StopSessionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        let Some(handle) = self.lookup_actor(&session.id)? else {
            return Ok(StopSessionResponse {
                session: contract_session(session),
            });
        };
        self.stop_actor(handle).await
    }

    /// Unloads one actor and removes only the Ora-owned session row.
    pub(crate) async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<DeleteSessionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(session_id)?;
        if let Some(handle) = self.lookup_actor(&session.id)? {
            self.stop_actor(handle).await?;
        }
        let deleted = SqliteSessionRepository::new(self.inner.pool.clone())
            .soft_delete_session(&session.id, self.inner.clock.now_timestamp_millis())
            .map_err(|source| BackendError::internal("failed to delete agent session", source))?;
        if !deleted {
            return Err(session_not_found(session_id));
        }
        self.actors_write()?.remove(&session.id);
        crate::session_history::remove_session_histories(
            &self.inner.sessions_root,
            [session.id.clone()],
        );
        Ok(DeleteSessionResponse {
            session_id: session.id.to_string(),
        })
    }

    /// Waits for an actor to unload its provider session and persist the stopped state.
    async fn stop_actor(
        &self,
        handle: RuntimeActorHandle,
    ) -> Result<StopSessionResponse, BackendError> {
        let (response_sender, response) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Stop {
                response: response_sender,
            })
            .map_err(runtime_unavailable_with)?;
        response.await.map_err(runtime_unavailable_with)?
    }

    /// Loads one non-deleted Ora session from durable storage.
    fn find_session(&self, session_id: &str) -> Result<Session, BackendError> {
        SqliteSessionRepository::new(self.inner.pool.clone())
            .find_session(&SessionId::new(session_id))
            .map_err(|source| BackendError::internal("failed to load session", source))?
            .ok_or_else(|| session_not_found(session_id))
    }

    /// Returns the live actor or restores one lazily after an application restart.
    fn actor_for(&self, session: Session) -> Result<RuntimeActorHandle, BackendError> {
        if let Some(handle) = self.lookup_actor(&session.id)? {
            return Ok(handle);
        }
        let cwd = resolve_task_cwd(&self.inner.pool, &session.task_id)?;
        let connection = self.inner.connections.for_agent(session.agent_cli);
        let mut opened = self.open_recorder(&session)?;
        let session = match opened.failure.take() {
            Some(reason) => self.settle_record(session, RecordOutcome::JustFailed { reason }),
            None => session,
        };
        let handoff_pending = opened.handoff_pending;
        self.insert_actor(
            session,
            cwd,
            connection,
            None,
            opened.recorder,
            handoff_pending,
        )
    }

    /// Reads the in-memory actor registry without creating a provider-side session.
    fn lookup_actor(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RuntimeActorHandle>, BackendError> {
        self.inner
            .actors
            .read()
            .map(|actors| actors.get(session_id).cloned())
            .map_err(|_poisoned| runtime_unavailable())
    }

    /// Installs exactly one actor for an Ora session under the lifecycle lock.
    fn insert_actor(
        &self,
        session: Session,
        cwd: PathBuf,
        connection: ConnectionSupervisor,
        channel: Option<SessionChannel>,
        recorder: SessionRecorder,
        handoff_pending: bool,
    ) -> Result<RuntimeActorHandle, BackendError> {
        let mut actors = self.actors_write()?;
        if let Some(handle) = actors.get(&session.id) {
            return Ok(handle.clone());
        }
        let (commands, receiver) = mpsc::unbounded_channel();
        let handle = RuntimeActorHandle { commands };
        actors.insert(session.id.clone(), handle.clone());
        tokio::spawn(
            RuntimeActor {
                session,
                cwd,
                repository: SqliteSessionRepository::new(self.inner.pool.clone()),
                clock: self.inner.clock,
                connection,
                channel,
                commands: receiver,
                recorder,
                sessions_root: self.inner.sessions_root.clone(),
                handoff_pending,
            }
            .run(),
        );
        Ok(handle)
    }

    /// Converts registry poisoning into the stable runtime-unavailable contract.
    fn actors_write(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, HashMap<SessionId, RuntimeActorHandle>>, BackendError>
    {
        self.inner
            .actors
            .write()
            .map_err(|_poisoned| runtime_unavailable())
    }
}

/// Releases a provider session Ora created and then decided not to keep.
///
/// The actor has its own detach for a binding it owns; this is the counterpart
/// for one that never reached an actor, so it reads the connection from the
/// channel rather than from a session row that does not name it yet.
async fn close_provider_session(provider: &ProviderSession) {
    use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
    use ora_contracts::acp::session::{CloseSessionRequest, CloseSessionResponse};
    use tokio::time::timeout;

    let connection = &provider.channel.connection;
    if !connection.close_session_supported {
        return;
    }
    let _ = timeout(
        CANCELLATION_GRACE,
        connection.client.request::<_, CloseSessionResponse>(
            AGENT_METHOD_NAMES.session_close,
            &CloseSessionRequest::new(provider.agent_session_id.clone()),
        ),
    )
    .await;
}

/// Maps the transport CLI identity onto the stable persisted one.
fn domain_agent_cli(agent_cli: ora_contracts::AgentCli) -> AgentCli {
    match agent_cli {
        ora_contracts::AgentCli::OpenCode => AgentCli::OpenCode,
        ora_contracts::AgentCli::Nga => AgentCli::Nga,
        ora_contracts::AgentCli::CodeAgentCli => AgentCli::CodeAgentCli,
    }
}

/// Builds the refusal returned while a session's history cannot be extended.
fn history_degraded() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
        "session history could not be recorded and must be resumed first",
    )
}

/// Restores durable lifecycle truth before the managed connection starts.
fn reconcile_running_sessions(
    pool: &RepositoryPool,
    clock: SystemClock,
) -> Result<(), BackendError> {
    let repository = SqliteSessionRepository::new(pool.clone());
    for session in repository
        .list_sessions()
        .map_err(|source| BackendError::internal("failed to reconcile sessions", source))?
    {
        if session.status == SessionStatus::Running {
            repository
                .update_session(
                    session.with_status(SessionStatus::Stopped, clock.now_timestamp_millis()),
                )
                .map_err(|source| BackendError::internal("failed to reconcile sessions", source))?;
        }
    }
    Ok(())
}

/// Extracts the latest setup command catalog while preserving other updates for the first prompt.
async fn collect_setup_commands(channel: &mut SessionChannel) -> Vec<AvailableCommand> {
    let mut available_commands = Vec::new();
    loop {
        // ACP sends setup updates before the response, but the shared router runs
        // independently and may need one short scheduling window to deliver them.
        let Ok(Some(notification)) =
            tokio::time::timeout(Duration::from_millis(10), channel.updates.recv()).await
        else {
            break;
        };
        if let SessionUpdate::AvailableCommandsUpdate(update) = &notification.update {
            // Command updates replace the full catalog, so the last setup value wins.
            available_commands = update.available_commands.clone();
        } else {
            channel.pending_updates.push_back(notification);
        }
    }
    available_commands
}

mod actor;
mod connection;
mod models;
mod routing;
mod stream;
mod support;

pub use stream::SessionEventStream;
use support::*;

use crate::clock::SystemClock;
use crate::task::resolve_task_cwd;
use crate::{BackendError, BackendErrorKind};
use connection::{ConnectionSupervisor, ConnectionSupervisors};
use ora_application::{Clock, SessionIdGenerator, SessionRepository, UuidSessionIdGenerator};
use ora_contracts::{
    AgentCli as ContractAgentCli, CreateSessionRequest, CreateSessionResponse,
    DeleteSessionResponse, LoadSessionEvent, LoadSessionRequest, PromptSessionEvent,
    PromptSessionRequest, RespondToPermissionRequest, RespondToPermissionResponse,
    StopSessionRequest, StopSessionResponse,
};
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{AgentCli, AuditFields, Session, SessionId, SessionStatus, TaskId};
use ora_logging::ora_debug;
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
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

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
        text: String,
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

/// Backend-only resolution of one Ora session to its private agent identifier and worktree cwd.
///
/// This is the surface the Desktop dashboard uses to resolve an Ora session id into a
/// concrete trace file path. It carries the agent session identifier, which is deliberately
/// omitted from the frontend-facing `ContractSession`; it never crosses the Tauri/Web boundary
/// and is consumed only by Desktop backend code that writes the dashboard locator file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLocator {
    /// The private provider-side session identifier owned by the agent CLI.
    pub agent_session_id: String,
    /// The persisted CLI selection, in the frontend-facing wire form Desktop already uses.
    pub agent_cli: ContractAgentCli,
    /// The authoritative worktree working directory resolved from the session's task.
    pub cwd: PathBuf,
    /// The user home directory, the root under which each agent writes its trace artifacts.
    pub home_directory: PathBuf,
}

struct RuntimeActor {
    session: Session,
    cwd: PathBuf,
    repository: SqliteSessionRepository,
    clock: SystemClock,
    connection: ConnectionSupervisor,
    channel: Option<SessionChannel>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
}

impl AgentRuntimeManager {
    /// Builds the manager, reconciles stale rows, and immediately starts the shared supervisor.
    pub(crate) fn new(
        pool: RepositoryPool,
        home_directory: PathBuf,
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
        use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
        use ora_contracts::acp::session::{NewSessionRequest, NewSessionResponse};
        use tokio::time::timeout;

        let _lifecycle = self.inner.lifecycle.lock().await;
        let agent_cli = match request.agent_cli {
            ora_contracts::AgentCli::OpenCode => AgentCli::OpenCode,
            ora_contracts::AgentCli::Nga => AgentCli::Nga,
            ora_contracts::AgentCli::CodeAgentCli => AgentCli::CodeAgentCli,
        };
        let cwd = resolve_task_cwd(&self.inner.pool, &TaskId::new(request.task_id.clone()))?;
        let supervisor = self.inner.connections.for_agent(agent_cli);
        let connection = supervisor.current()?;
        let response = timeout(
            SESSION_SETUP_TIMEOUT,
            connection.client.request::<_, NewSessionResponse>(
                AGENT_METHOD_NAMES.session_new,
                &NewSessionRequest::new(&cwd),
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
        let channel = supervisor.open_session_channel(response.session_id.0.as_ref())?;
        let now = self.inner.clock.now_timestamp_millis();
        let session = Session::new(
            UuidSessionIdGenerator::new().generate_session_id(),
            TaskId::new(request.task_id),
            agent_cli,
            response.session_id.to_string(),
            SessionStatus::Running,
            AuditFields::new(now, now, false),
        );
        SqliteSessionRepository::new(self.inner.pool.clone())
            .create_session(session.clone())
            .map_err(|_| {
                runtime_internal(
                    "session_repository_error",
                    "failed to persist agent CLI session",
                )
            })?;
        ora_debug!(
            session_id = %session.id,
            agent_session_id = %session.agent_session_id,
            "session created",
        );
        self.insert_actor(session.clone(), cwd, supervisor, Some(channel))?;
        Ok(CreateSessionResponse {
            session: contract_session(session),
        })
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
            .map_err(|_| runtime_unavailable())?;
        accepted.await.map_err(|_| runtime_unavailable())??;
        Ok(SessionEventStream::new(
            events,
            handle.commands,
            operation_id,
        ))
    }

    /// Starts one text-only prompt while preserving cross-session concurrency.
    pub(crate) async fn prompt_session(
        &self,
        request: PromptSessionRequest,
    ) -> Result<SessionEventStream<PromptSessionEvent>, BackendError> {
        let text = request.text.trim().to_string();
        if text.is_empty() {
            return Err(BackendError::new(
                BackendErrorKind::BadRequest,
                "prompt_empty",
                "prompt text must not be empty",
            ));
        }
        if text.len() > MAX_PROMPT_BYTES {
            return Err(BackendError::new(
                BackendErrorKind::BadRequest,
                "prompt_too_large",
                "prompt text exceeds 1 MiB",
            ));
        }
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        if session.status != SessionStatus::Running {
            return Err(session_stopped());
        }
        let handle = self.actor_for(session)?;
        let operation_id = self.inner.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let (events_sender, events) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
        let (accepted_sender, accepted) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Prompt {
                operation_id,
                text,
                events: events_sender,
                accepted: accepted_sender,
            })
            .map_err(|_| runtime_unavailable())?;
        accepted.await.map_err(|_| runtime_unavailable())??;
        Ok(SessionEventStream::new(
            events,
            handle.commands,
            operation_id,
        ))
    }

    /// Routes one opaque permission response to the actor that owns the logical session.
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
            .map_err(|_| runtime_unavailable())?;
        response.await.map_err(|_| runtime_unavailable())?
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
            .map_err(|_| {
                runtime_internal("session_repository_error", "failed to delete agent session")
            })?;
        if !deleted {
            return Err(session_not_found(session_id));
        }
        self.actors_write()?.remove(&session.id);
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
            .map_err(|_| runtime_unavailable())?;
        response.await.map_err(|_| runtime_unavailable())?
    }

    /// Resolves one Ora session id to its private agent session identifier and worktree cwd.
    ///
    /// Backend-only: the returned `agent_session_id` is never exposed to the frontend. The
    /// Desktop dashboard command consumes it to locate the agent-written trace file and writes
    /// only a resolved file path into the locator it hands the embedded dashboard.
    pub fn resolve_session_locator(
        &self,
        session_id: &str,
    ) -> Result<SessionLocator, BackendError> {
        let session = self.find_session(session_id)?;
        let cwd = resolve_task_cwd(&self.inner.pool, &session.task_id)?;
        Ok(SessionLocator {
            agent_session_id: session.agent_session_id.clone(),
            agent_cli: contract_agent_cli(session.agent_cli),
            cwd,
            home_directory: self.inner.home_directory.clone(),
        })
    }

    /// Loads one non-deleted Ora session from durable storage.
    fn find_session(&self, session_id: &str) -> Result<Session, BackendError> {
        SqliteSessionRepository::new(self.inner.pool.clone())
            .find_session(&SessionId::new(session_id))
            .map_err(|_| runtime_internal("session_repository_error", "failed to load session"))?
            .ok_or_else(|| session_not_found(session_id))
    }

    /// Returns the live actor or restores one lazily after an application restart.
    fn actor_for(&self, session: Session) -> Result<RuntimeActorHandle, BackendError> {
        if let Some(handle) = self.lookup_actor(&session.id)? {
            return Ok(handle);
        }
        let cwd = resolve_task_cwd(&self.inner.pool, &session.task_id)?;
        let connection = self.inner.connections.for_agent(session.agent_cli);
        self.insert_actor(session, cwd, connection, None)
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
            .map_err(|_| runtime_unavailable())
    }

    /// Installs exactly one actor for an Ora session under the lifecycle lock.
    fn insert_actor(
        &self,
        session: Session,
        cwd: PathBuf,
        connection: ConnectionSupervisor,
        channel: Option<SessionChannel>,
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
        self.inner.actors.write().map_err(|_| runtime_unavailable())
    }
}

/// Restores durable lifecycle truth before the managed connection starts.
fn reconcile_running_sessions(
    pool: &RepositoryPool,
    clock: SystemClock,
) -> Result<(), BackendError> {
    let repository = SqliteSessionRepository::new(pool.clone());
    for session in repository
        .list_sessions()
        .map_err(|_| runtime_internal("session_repository_error", "failed to reconcile sessions"))?
    {
        if session.status == SessionStatus::Running {
            repository
                .update_session(
                    session.with_status(SessionStatus::Stopped, clock.now_timestamp_millis()),
                )
                .map_err(|_| {
                    runtime_internal("session_repository_error", "failed to reconcile sessions")
                })?;
        }
    }
    Ok(())
}

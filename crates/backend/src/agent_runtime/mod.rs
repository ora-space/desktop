mod actor;
mod paths;
mod stream;

pub use stream::SessionEventStream;

use crate::clock::SystemClock;
use crate::{BackendError, BackendErrorKind};
use gitlancer::git::worktree::ResolveWorktreeByBranchRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository};
use ora_acp::{AcpClient, AcpControl, AcpPeer};
use ora_application::{
    Clock, ProjectRepository, SessionIdGenerator, SessionRepository, TaskRepository,
    UuidSessionIdGenerator, WorktreeRepository,
};
use ora_contracts::acp::common::SessionId as AcpSessionId;
use ora_contracts::acp::content::ContentBlock;
use ora_contracts::acp::initialization::{
    Implementation, InitializeRequest, InitializeResponse, ProtocolVersion,
};
use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
use ora_contracts::acp::notification::CancelNotification;
use ora_contracts::acp::permission::{
    PermissionOptionId, RequestPermissionOutcome, RequestPermissionResponse,
    SelectedPermissionOutcome,
};
use ora_contracts::acp::prompt::{PromptRequest, PromptResponse};
use ora_contracts::acp::session::{
    LoadSessionRequest as AcpLoadSessionRequest, LoadSessionResponse, NewSessionRequest,
    NewSessionResponse,
};
use ora_contracts::acp::session_mode::{
    SetSessionModeRequest as AcpSetSessionModeRequest,
    SetSessionModeResponse as AcpSetSessionModeResponse,
};
use ora_contracts::{
    AgentCli as ContractAgentCli, CreateSessionRequest, CreateSessionResponse,
    DeleteSessionResponse, LoadSessionEvent, LoadSessionRequest, PromptSessionEvent,
    PromptSessionRequest, RespondToPermissionRequest, RespondToPermissionResponse,
    Session as ContractSession, SessionPermissionRequest, SessionStatus as ContractSessionStatus,
    SetSessionModeRequest, SetSessionModeResponse, StopSessionRequest, StopSessionResponse,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteSessionRepository, SqliteTaskRepository,
    SqliteWorktreeRepository,
};
use ora_domain::{
    AgentCli, AuditFields, Session, SessionId, SessionStatus, TaskId, WorktreeActivity,
};
use ora_logging::ora_debug;
use ora_process::{
    ManagedProcess, ProcessSpawner, ProcessSpec, TokioManagedProcess, TokioProcessSpawner,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
const SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const CANCELLATION_GRACE: Duration = Duration::from_secs(5);
const CONTRACT_QUEUE_CAPACITY: usize = 256;
const MAX_PROMPT_BYTES: usize = 16 * 1024 * 1024;

/// Coordinates one actor per Ora session while keeping provider process ownership in the backend.
#[derive(Clone)]
pub(crate) struct AgentRuntimeManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    pool: RepositoryPool,
    home_directory: PathBuf,
    opencode_path: PathBuf,
    actors: RwLock<HashMap<SessionId, RuntimeActorHandle>>,
    lifecycle: tokio::sync::Mutex<()>,
    next_operation_id: AtomicU64,
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
    SetMode {
        mode_id: ora_contracts::acp::session_mode::SessionModeId,
        response: oneshot::Sender<Result<SetSessionModeResponse, BackendError>>,
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
    home_directory: PathBuf,
    opencode_path: PathBuf,
    repository: SqliteSessionRepository,
    clock: SystemClock,
    process: Option<AgentProcess>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
}

struct AgentProcess {
    child: TokioManagedProcess,
    client: AcpClient<ChildStdin>,
    updates: mpsc::Receiver<ora_contracts::acp::notification::SessionNotification>,
    control: mpsc::UnboundedReceiver<AcpControl>,
    load_session_supported: bool,
}

impl AgentRuntimeManager {
    /// Builds the process manager and reconciles stale running rows without spawning providers.
    pub(crate) fn new(
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Result<Self, BackendError> {
        let repository = SqliteSessionRepository::new(pool.clone());
        let mut reconciled = 0u64;
        for session in repository.list_sessions().map_err(|error| {
            runtime_internal(
                "session_repository_error",
                format!("failed to reconcile sessions: {error:?}"),
            )
        })? {
            if session.status == SessionStatus::Running {
                repository
                    .update_session(
                        session.with_status(SessionStatus::Stopped, clock.now_timestamp_millis()),
                    )
                    .map_err(|_| {
                        runtime_internal("session_repository_error", "failed to reconcile sessions")
                    })?;
                reconciled += 1;
            }
        }
        if reconciled > 0 {
            ora_debug!(
                reconciled_count = reconciled,
                "reconciled stale running sessions"
            );
        }
        let opencode_path = resolve_opencode_path(&home_directory)?;
        Ok(Self {
            inner: Arc::new(ManagerInner {
                pool,
                home_directory,
                opencode_path,
                actors: RwLock::new(HashMap::new()),
                lifecycle: tokio::sync::Mutex::new(()),
                next_operation_id: AtomicU64::new(1),
                clock,
            }),
        })
    }

    /// Creates a provider session only after cwd resolution and the ACP setup handshake succeed.
    pub(crate) async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, BackendError> {
        let agent_cli = domain_agent_cli(request.agent_cli);
        let cwd = resolve_task_cwd(&self.inner.pool, &TaskId::new(request.task_id.clone()))?;
        let process = spawn_initialized_process(
            agent_cli,
            &cwd,
            &self.inner.home_directory,
            &self.inner.opencode_path,
        )
        .await?;
        let response = timeout(
            SESSION_SETUP_TIMEOUT,
            process.client.request::<_, NewSessionResponse>(
                AGENT_METHOD_NAMES.session_new,
                &NewSessionRequest::new(&cwd),
            ),
        )
        .await
        .map_err(|_| runtime_internal("agent_start_timeout", "agent session creation timed out"))?
        .map_err(map_acp_error)?;
        let now = self.inner.clock.now_timestamp_millis();
        let session = Session::new(
            UuidSessionIdGenerator::new().generate_session_id(),
            TaskId::new(request.task_id),
            agent_cli,
            response.session_id.0.to_string(),
            SessionStatus::Running,
            AuditFields::new(now, now, false),
        );
        SqliteSessionRepository::new(self.inner.pool.clone())
            .create_session(session.clone())
            .map_err(|_| {
                runtime_internal(
                    "session_repository_error",
                    "failed to persist agent session",
                )
            })?;
        ora_debug!(
            session_id = %session.id,
            agent_session_id = %session.agent_session_id,
            "session created",
        );
        self.insert_actor(session.clone(), cwd, Some(process))?;
        Ok(CreateSessionResponse {
            session: contract_session(session),
            modes: response.modes,
        })
    }

    /// Starts an explicit ACP load stream for one persisted Ora session.
    pub(crate) async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> Result<SessionEventStream<LoadSessionEvent>, BackendError> {
        // Registration is serialized with deletion so an observer of the old row cannot enqueue
        // work after deletion has already stopped and detached its actor.
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
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "session runtime is unavailable",
                )
            })?;
        accepted.await.map_err(|_| {
            runtime_internal("agent_runtime_unavailable", "session runtime stopped")
        })??;
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
                BackendErrorKind::BadRequest,
                "prompt_empty",
                "prompt must contain text or media",
            ));
        }
        let prompt_bytes = serde_json::to_vec(&prompt)
            .map_err(|_| runtime_internal("prompt_encoding_failed", "failed to encode prompt"))?
            .len();
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(BackendError::new(
                BackendErrorKind::BadRequest,
                "prompt_too_large",
                "prompt exceeds 16 MiB",
            ));
        }
        // Only command acceptance is serialized; active prompts on separate sessions remain
        // concurrent while deletion cannot pass this registration point.
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        if session.status != SessionStatus::Running {
            return Err(BackendError::new(
                BackendErrorKind::Conflict,
                "session_stopped",
                "session must be loaded before prompting",
            ));
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
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "session runtime is unavailable",
                )
            })?;
        accepted.await.map_err(|_| {
            runtime_internal("agent_runtime_unavailable", "session runtime stopped")
        })??;
        Ok(SessionEventStream::new(
            events,
            handle.commands,
            operation_id,
        ))
    }

    /// Changes the provider mode for one running, idle session.
    pub(crate) async fn set_session_mode(
        &self,
        request: SetSessionModeRequest,
    ) -> Result<SetSessionModeResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(&request.session_id)?;
        if session.status != SessionStatus::Running {
            return Err(BackendError::new(
                BackendErrorKind::Conflict,
                "session_stopped",
                "session must be loaded before changing mode",
            ));
        }
        let handle = self.actor_for(session)?;
        let (response_sender, response) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::SetMode {
                mode_id: request.mode_id,
                response: response_sender,
            })
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "session runtime is unavailable",
                )
            })?;
        response
            .await
            .map_err(|_| runtime_internal("agent_runtime_unavailable", "session runtime stopped"))?
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
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "session runtime is unavailable",
                )
            })?;
        response
            .await
            .map_err(|_| runtime_internal("agent_runtime_unavailable", "session runtime stopped"))?
    }

    /// Stops a provider process while preserving the Ora session for a later explicit load.
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
        let (response_sender, response) = oneshot::channel();
        handle
            .commands
            .send(RuntimeCommand::Stop {
                response: response_sender,
            })
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "session runtime is unavailable",
                )
            })?;
        response
            .await
            .map_err(|_| runtime_internal("agent_runtime_unavailable", "session runtime stopped"))?
    }

    /// Stops any live process and removes the Ora row under one lifecycle exclusion guard.
    pub(crate) async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<DeleteSessionResponse, BackendError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let session = self.find_session(session_id)?;
        if let Some(handle) = self.lookup_actor(&session.id)? {
            let (response_sender, response) = oneshot::channel();
            handle
                .commands
                .send(RuntimeCommand::Stop {
                    response: response_sender,
                })
                .map_err(|_| {
                    runtime_internal(
                        "agent_runtime_unavailable",
                        "session runtime is unavailable",
                    )
                })?;
            response.await.map_err(|_| {
                runtime_internal("agent_runtime_unavailable", "session runtime stopped")
            })??;
        }
        let deleted = SqliteSessionRepository::new(self.inner.pool.clone())
            .soft_delete_session(&session.id, self.inner.clock.now_timestamp_millis())
            .map_err(|_| {
                runtime_internal("session_repository_error", "failed to delete agent session")
            })?;
        if !deleted {
            return Err(BackendError::new(
                BackendErrorKind::NotFound,
                "session_not_found",
                format!("session not found: {session_id}"),
            ));
        }
        self.inner
            .actors
            .write()
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "runtime registry is unavailable",
                )
            })?
            .remove(&session.id);
        Ok(DeleteSessionResponse {
            session_id: session.id.to_string(),
        })
    }

    /// Finds one visible persisted session and preserves its private provider id inside the backend.
    fn find_session(&self, session_id: &str) -> Result<Session, BackendError> {
        SqliteSessionRepository::new(self.inner.pool.clone())
            .find_session(&SessionId::new(session_id))
            .map_err(|_| runtime_internal("session_repository_error", "failed to load session"))?
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorKind::NotFound,
                    "session_not_found",
                    format!("session not found: {session_id}"),
                )
            })
    }

    /// Returns or starts the one actor generation responsible for a persisted session.
    fn actor_for(&self, session: Session) -> Result<RuntimeActorHandle, BackendError> {
        if let Some(handle) = self.lookup_actor(&session.id)? {
            return Ok(handle);
        }
        let cwd = resolve_task_cwd(&self.inner.pool, &session.task_id)?;
        self.insert_actor(session, cwd, None)
    }

    /// Looks up an actor without holding the registry lock across asynchronous work.
    fn lookup_actor(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RuntimeActorHandle>, BackendError> {
        self.inner
            .actors
            .read()
            .map(|actors| actors.get(session_id).cloned())
            .map_err(|_| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    "runtime registry is unavailable",
                )
            })
    }

    /// Inserts one actor only if another generation did not win the same-session race.
    fn insert_actor(
        &self,
        session: Session,
        cwd: PathBuf,
        process: Option<AgentProcess>,
    ) -> Result<RuntimeActorHandle, BackendError> {
        let mut actors = self.inner.actors.write().map_err(|_| {
            runtime_internal(
                "agent_runtime_unavailable",
                "runtime registry is unavailable",
            )
        })?;
        if let Some(handle) = actors.get(&session.id) {
            return Ok(handle.clone());
        }
        let (commands, receiver) = mpsc::unbounded_channel();
        let handle = RuntimeActorHandle { commands };
        actors.insert(session.id.clone(), handle.clone());
        tokio::spawn(
            RuntimeActor {
                opencode_path: self.inner.opencode_path.clone(),
                session,
                cwd,
                home_directory: self.inner.home_directory.clone(),
                repository: SqliteSessionRepository::new(self.inner.pool.clone()),
                clock: self.inner.clock,
                process,
                commands: receiver,
            }
            .run(),
        );
        Ok(handle)
    }
}

/// Starts one CLI, drains stderr, and performs the capability handshake.
async fn spawn_initialized_process(
    agent_cli: AgentCli,
    cwd: &Path,
    home_directory: &Path,
    opencode_path: &Path,
) -> Result<AgentProcess, BackendError> {
    let executable = paths::executable_for(agent_cli, home_directory, opencode_path);
    if !executable.is_file() {
        return Err(BackendError::new(
            BackendErrorKind::NotFound,
            "agent_cli_not_found",
            format!("agent CLI executable not found: {}", executable.display()),
        ));
    }
    ora_debug!(executable = %executable.display(), cwd = %cwd.display(), "spawning agent process");
    let mut child = TokioProcessSpawner::new()
        .spawn(ProcessSpec::new(executable).arg("acp").cwd(cwd))
        .map_err(|_| runtime_internal("agent_start_failed", "failed to start agent CLI"))?;
    let stdin = child
        .take_stdin()
        .ok_or_else(|| runtime_internal("agent_start_failed", "agent stdin pipe is unavailable"))?;
    let stdout = child.take_stdout().ok_or_else(|| {
        runtime_internal("agent_start_failed", "agent stdout pipe is unavailable")
    })?;
    if let Some(stderr) = child.take_stderr() {
        tokio::spawn(drain_stderr(stderr));
    }
    let peer = AcpPeer::spawn(stdout, stdin);
    let initialize = InitializeRequest::new(ProtocolVersion(1))
        .client_info(Implementation::new("ora", env!("CARGO_PKG_VERSION")));
    let response = timeout(
        INITIALIZE_TIMEOUT,
        peer.client
            .request::<_, InitializeResponse>(AGENT_METHOD_NAMES.initialize, &initialize),
    )
    .await
    .map_err(|_| runtime_internal("agent_initialize_timeout", "agent initialization timed out"))?
    .map_err(map_acp_error)?;
    ora_debug!(
        load_session_supported = response.agent_capabilities.load_session,
        "initialize handshake completed",
    );
    let (client, updates, control) = peer.into_parts();
    Ok(AgentProcess {
        child,
        client,
        updates,
        control,
        load_session_supported: response.agent_capabilities.load_session,
    })
}

/// Continuously drains stderr and retains only a bounded tail inside the task.
async fn drain_stderr(mut stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncReadExt;
    let mut tail = Vec::with_capacity(64 * 1024);
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                tail.extend_from_slice(&buffer[..read]);
                if tail.len() > 64 * 1024 {
                    tail.drain(..tail.len() - 64 * 1024);
                }
            }
        }
    }
}

/// Resolves the authoritative task worktree path through persisted ownership and Git metadata.
pub(crate) fn resolve_task_cwd(
    pool: &RepositoryPool,
    task_id: &TaskId,
) -> Result<PathBuf, BackendError> {
    let task = SqliteTaskRepository::new(pool.clone())
        .find_task(task_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    if task.worktree_id.is_none() {
        let project = SqliteProjectRepository::new(pool.clone())
            .find_project(&task.project_id)
            .map_err(|_| task_project_root_unavailable())?
            .ok_or_else(task_project_root_unavailable)?;
        let cwd = PathBuf::from(project.root_path);
        return if cwd.is_dir() {
            Ok(cwd)
        } else {
            Err(task_project_root_unavailable())
        };
    }

    let worktree_id = task.worktree_id.ok_or_else(task_worktree_unavailable)?;
    let worktree = SqliteWorktreeRepository::new(pool.clone())
        .find_worktree(&worktree_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    if worktree.task_id != task.id || worktree.activity != WorktreeActivity::Active {
        return Err(task_worktree_unavailable());
    }
    let branch_name = worktree.branch_name.ok_or_else(task_worktree_unavailable)?;
    let project = SqliteProjectRepository::new(pool.clone())
        .find_project(&task.project_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    let repository = Repository::new(RepoRoot::new(project.root_path));
    let handle = Git::new(CliGitRunner)
        .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
            repository: &repository,
            branch_name: &branch_name,
        })
        .map_err(|_| task_worktree_unavailable())?;
    let cwd = handle.worktree_root().as_path().to_path_buf();
    if !cwd.is_dir() {
        return Err(task_worktree_unavailable());
    }
    Ok(cwd)
}

/// Sends one validated permission choice and prevents duplicate responses.
async fn respond_permission(
    client: &AcpClient<ChildStdin>,
    request: RespondToPermissionRequest,
    permissions: &mut HashMap<String, (ora_contracts::acp::rpc::RequestId, Vec<String>)>,
) -> Result<RespondToPermissionResponse, BackendError> {
    let (request_id, options) = permissions
        .remove(&request.permission_request_id)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorKind::Conflict,
                "permission_request_not_pending",
                "permission request is not pending",
            )
        })?;
    if !options.contains(&request.option_id) {
        permissions.insert(request.permission_request_id, (request_id, options));
        return Err(BackendError::new(
            BackendErrorKind::BadRequest,
            "permission_option_invalid",
            "permission option does not belong to this request",
        ));
    }
    let selected_option = request.option_id.clone();
    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
        PermissionOptionId::new(request.option_id),
    ));
    client
        .respond(&request_id, &RequestPermissionResponse::new(outcome))
        .await
        .map_err(map_acp_error)?;
    ora_debug!(option_id = %selected_option, "permission responded");
    Ok(RespondToPermissionResponse {})
}

/// Maps the private domain snapshot into the frontend-safe view.
fn contract_session(session: Session) -> ContractSession {
    ContractSession {
        id: session.id.to_string(),
        task_id: session.task_id.to_string(),
        agent_cli: contract_agent_cli(session.agent_cli),
        status: match session.status {
            SessionStatus::Running => ContractSessionStatus::Running,
            SessionStatus::Stopped => ContractSessionStatus::Stopped,
        },
    }
}

/// Converts the public CLI selection into its persistence representation.
fn domain_agent_cli(agent_cli: ContractAgentCli) -> AgentCli {
    match agent_cli {
        ContractAgentCli::OpenCode => AgentCli::OpenCode,
        ContractAgentCli::Nga => AgentCli::Nga,
        ContractAgentCli::CodeAgentCli => AgentCli::CodeAgentCli,
    }
}

/// Converts the persistence CLI selection into its public wire representation.
fn contract_agent_cli(agent_cli: AgentCli) -> ContractAgentCli {
    match agent_cli {
        AgentCli::OpenCode => ContractAgentCli::OpenCode,
        AgentCli::Nga => ContractAgentCli::Nga,
        AgentCli::CodeAgentCli => ContractAgentCli::CodeAgentCli,
    }
}

/// Resolves OpenCode through the Windows executable lookup mechanism once at startup.
#[cfg(windows)]
fn resolve_opencode_path(_home_directory: &Path) -> Result<PathBuf, BackendError> {
    let output = std::process::Command::new("where.exe")
        .arg("opencode")
        .output()
        .map_err(|_| runtime_internal("opencode_resolution_failed", "failed to run where.exe"))?;
    if !output.status.success() {
        return Err(runtime_internal(
            "opencode_not_found",
            "opencode executable not found on PATH",
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|line| {
            let lower = line.to_lowercase();
            lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat")
        })
        .or_else(|| stdout.lines().next())
        .map(|path| PathBuf::from(path.trim()))
        .ok_or_else(|| {
            runtime_internal(
                "opencode_not_found",
                "opencode executable not found on PATH",
            )
        })
}

/// Resolves OpenCode from its fixed per-user Unix installation directory once at startup.
#[cfg(unix)]
fn resolve_opencode_path(home_directory: &Path) -> Result<PathBuf, BackendError> {
    // Process creation owns the existence check so the backend can still expose non-agent
    // capabilities when a provider has not been installed for this user.
    Ok(home_directory
        .join(".opencode")
        .join("bin")
        .join("opencode"))
}

/// Builds the stable error used when ownership cannot resolve a live Git worktree.
fn task_worktree_unavailable() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "task_worktree_unavailable",
        "task worktree is unavailable",
    )
}

/// Builds the stable error used when a project-root task cannot resolve its directory.
fn task_project_root_unavailable() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "task_project_root_unavailable",
        "task project root is unavailable",
    )
}

/// Sanitizes ACP peer details behind one transport-neutral protocol failure.
fn map_acp_error(error: ora_acp::AcpError) -> BackendError {
    runtime_internal("agent_protocol_error", error.to_string())
}

/// Builds a private runtime failure with a stable public code.
fn runtime_internal(code: &'static str, message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorKind::Internal, code, message)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::resolve_opencode_path;
    use super::resolve_task_cwd;
    use ora_application::{ProjectRepository, TaskRepository};
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, SqliteTaskRepository,
        default_migration_catalog,
    };
    use ora_domain::{AuditFields, Project, ProjectId, Task, TaskId, TaskStatus};
    use pretty_assertions::assert_eq;
    use std::fs;
    #[cfg(unix)]
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Verifies Unix builds resolve OpenCode from the configured user's fixed installation path.
    #[cfg(unix)]
    #[test]
    fn resolves_unix_opencode_path_from_home_directory() {
        let home_directory = PathBuf::from("users").join("demo");

        assert_eq!(
            resolve_opencode_path(&home_directory).expect("resolve OpenCode path"),
            home_directory
                .join(".opencode")
                .join("bin")
                .join("opencode")
        );
    }

    /// Verifies tasks without a linked worktree start agents in their project's root directory.
    #[test]
    fn resolves_project_root_for_tasks_without_worktrees() {
        let temp_dir = TempDir::new().expect("create temporary directory");
        let project_root = temp_dir.path().join("project-root");
        fs::create_dir_all(&project_root).expect("create project root");
        let database_path = temp_dir.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool");
        let project = Project::new(
            ProjectId::new("project-1"),
            "Project",
            project_root.to_string_lossy(),
            AuditFields::new(1, 1, false),
        );
        SqliteProjectRepository::new(pool.clone())
            .create_project(project)
            .expect("persist project");
        SqliteTaskRepository::new(pool.clone())
            .create_task(Task::new(
                TaskId::new("task-1"),
                ProjectId::new("project-1"),
                "Project chat",
                TaskStatus::Doing,
                None,
                AuditFields::new(1, 1, false),
            ))
            .expect("persist task");

        assert_eq!(
            resolve_task_cwd(&pool, &TaskId::new("task-1")).expect("resolve project root cwd"),
            project_root,
        );
    }
}

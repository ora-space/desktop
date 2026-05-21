use crate::session::{SessionIdGenerator, SessionRepository};
use crate::task::TaskRepository;
use crate::terminal::ports::{
    TerminalAttachment, TerminalRuntime, TerminalRuntimeError, TerminalRuntimeRequest,
    TerminalRuntimeResult, TerminalStartupConfig,
};
use crate::worktree::WorktreeRepository;
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, SessionStatus, TerminalSessionStartup,
};
use ora_domain::{
    AgentId, AuditFields, Session as DomainSession, SessionId,
    SessionStatus as DomainSessionStatus, TaskId, WorktreeId,
};
use ora_logging::{ora_error, ora_info};
use ora_pty::{PtyLifecycleEvent, PtySessionId};
use std::path::{Path, PathBuf};

/// Handles terminal session startup while keeping PTY lifecycle orchestration out of adapters.
pub struct CreateTerminalSessionHandler<
    SessionRepositoryPort,
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    RuntimePort,
    IdGeneratorPort,
    ClockSource,
> {
    session_repository: SessionRepositoryPort,
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    runtime: RuntimePort,
    id_generator: IdGeneratorPort,
    startup_config: TerminalStartupConfig,
    clock: ClockSource,
}

impl<
    SessionRepositoryPort,
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    RuntimePort,
    IdGeneratorPort,
    ClockSource,
>
    CreateTerminalSessionHandler<
        SessionRepositoryPort,
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        RuntimePort,
        IdGeneratorPort,
        ClockSource,
    >
{
    /// Builds the terminal startup handler from application-owned dependencies.
    pub fn new(
        session_repository: SessionRepositoryPort,
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        runtime: RuntimePort,
        id_generator: IdGeneratorPort,
        startup_config: TerminalStartupConfig,
        clock: ClockSource,
    ) -> Self {
        Self {
            session_repository,
            task_repository,
            worktree_repository,
            runtime,
            id_generator,
            startup_config,
            clock,
        }
    }
}

impl<
    SessionRepositoryPort,
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    RuntimePort,
    IdGeneratorPort,
    ClockSource,
>
    CreateTerminalSessionHandler<
        SessionRepositoryPort,
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        RuntimePort,
        IdGeneratorPort,
        ClockSource,
    >
where
    SessionRepositoryPort: SessionRepository,
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    RuntimePort: TerminalRuntime,
    IdGeneratorPort: SessionIdGenerator,
    ClockSource: Clock,
{
    /// Persists a terminal session, starts the PTY runtime, and compensates startup failures.
    pub fn handle(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, ApplicationError> {
        let terminal = validate_terminal_request(&request)?;
        let task_id = TaskId::new(request.task_id);
        let task = self
            .task_repository
            .find_task(&task_id)
            .map_err(ApplicationError::from_task_repository_error)?
            .ok_or_else(|| ApplicationError::TaskNotFound {
                task_id: task_id.to_string(),
            })?;
        let worktree_id =
            task.worktree_id
                .clone()
                .ok_or_else(|| ApplicationError::WorktreeNotFound {
                    worktree_id: format!("task-{}-active-worktree", task.id),
                })?;

        validate_active_worktree(&self.worktree_repository, &task_id, &worktree_id)?;

        let now = self.clock.now_timestamp_millis();
        let session = DomainSession::new(
            self.id_generator.generate_session_id(),
            task.id,
            AgentId::terminal(),
            request.agent_session_id,
            map_contract_session_status(request.status),
            AuditFields::new(now, now, false),
        );
        let session = self
            .session_repository
            .create_session(session)
            .map_err(|error| {
                let error = ApplicationError::from_session_repository_error(error);
                log_terminal_failure("create_terminal_session", None, &error);
                error
            })?;

        let runtime_request = TerminalRuntimeRequest {
            session_id: PtySessionId::new(session.id.to_string()),
            cwd: worktree_path_for_task(&self.startup_config.work_dir, &task_id),
            program: self.startup_config.shell_program.clone(),
            args: Vec::new(),
            cols: terminal.cols,
            rows: terminal.rows,
        };
        let runtime_result = self.runtime.start_session(runtime_request);

        match runtime_result {
            Ok(TerminalRuntimeResult { .. }) => {
                log_terminal_success("create_terminal_session", &session.id);

                Ok(CreateSessionResponse {
                    session: map_session(session),
                })
            }
            Err(error) => self.compensate_terminal_startup_failure(session, error),
        }
    }

    /// Marks a persisted session stopped after PTY startup fails so diagnostics remain visible.
    fn compensate_terminal_startup_failure(
        &self,
        mut session: DomainSession,
        runtime_error: TerminalRuntimeError,
    ) -> Result<CreateSessionResponse, ApplicationError> {
        session.status = DomainSessionStatus::Stopped;
        session.audit_fields.updated_at = self.clock.now_timestamp_millis();
        let session_id = session.id.clone();
        self.session_repository
            .update_session(session)
            .map_err(|error| {
                let error = ApplicationError::from_session_repository_error(error);
                log_terminal_failure("create_terminal_session", Some(&session_id), &error);
                error
            })?;

        let error = map_terminal_runtime_error(runtime_error);
        log_terminal_failure("create_terminal_session", Some(&session_id), &error);

        Err(error)
    }
}

/// Handles terminal attachment without exposing PTY internals to adapters.
pub struct AttachTerminalSessionHandler<SessionRepositoryPort, RuntimePort> {
    session_repository: SessionRepositoryPort,
    runtime: RuntimePort,
}

impl<SessionRepositoryPort, RuntimePort>
    AttachTerminalSessionHandler<SessionRepositoryPort, RuntimePort>
{
    /// Builds the terminal attach handler from application-owned dependencies.
    pub fn new(session_repository: SessionRepositoryPort, runtime: RuntimePort) -> Self {
        Self {
            session_repository,
            runtime,
        }
    }
}

impl<SessionRepositoryPort, RuntimePort>
    AttachTerminalSessionHandler<SessionRepositoryPort, RuntimePort>
where
    SessionRepositoryPort: SessionRepository,
    RuntimePort: TerminalRuntime,
{
    /// Validates the addressed terminal session and returns replay plus a live runtime stream.
    pub fn handle(&self, session_id: String) -> Result<TerminalAttachment, ApplicationError> {
        let session = load_running_terminal_session(&self.session_repository, &session_id)?;

        self.runtime
            .attach_session(&PtySessionId::new(session.id.to_string()))
            .map_err(map_terminal_runtime_error)
    }
}

/// Handles terminal input without exposing PTY internals to adapters.
pub struct SendTerminalInputHandler<SessionRepositoryPort, RuntimePort> {
    session_repository: SessionRepositoryPort,
    runtime: RuntimePort,
}

impl<SessionRepositoryPort, RuntimePort>
    SendTerminalInputHandler<SessionRepositoryPort, RuntimePort>
{
    /// Builds the terminal input handler from application-owned dependencies.
    pub fn new(session_repository: SessionRepositoryPort, runtime: RuntimePort) -> Self {
        Self {
            session_repository,
            runtime,
        }
    }
}

impl<SessionRepositoryPort, RuntimePort>
    SendTerminalInputHandler<SessionRepositoryPort, RuntimePort>
where
    SessionRepositoryPort: SessionRepository,
    RuntimePort: TerminalRuntime,
{
    /// Sends raw terminal input into the running PTY session addressed by the caller.
    pub fn handle(&self, session_id: String, data: String) -> Result<(), ApplicationError> {
        let session = load_running_terminal_session(&self.session_repository, &session_id)?;

        self.runtime
            .send_input(&PtySessionId::new(session.id.to_string()), data)
            .map_err(map_terminal_runtime_error)
    }
}

/// Handles terminal resize without exposing PTY internals to adapters.
pub struct ResizeTerminalSessionHandler<SessionRepositoryPort, RuntimePort> {
    session_repository: SessionRepositoryPort,
    runtime: RuntimePort,
}

impl<SessionRepositoryPort, RuntimePort>
    ResizeTerminalSessionHandler<SessionRepositoryPort, RuntimePort>
{
    /// Builds the terminal resize handler from application-owned dependencies.
    pub fn new(session_repository: SessionRepositoryPort, runtime: RuntimePort) -> Self {
        Self {
            session_repository,
            runtime,
        }
    }
}

impl<SessionRepositoryPort, RuntimePort>
    ResizeTerminalSessionHandler<SessionRepositoryPort, RuntimePort>
where
    SessionRepositoryPort: SessionRepository,
    RuntimePort: TerminalRuntime,
{
    /// Applies one viewport resize to the running PTY session addressed by the caller.
    pub fn handle(&self, session_id: String, cols: u16, rows: u16) -> Result<(), ApplicationError> {
        let session = load_running_terminal_session(&self.session_repository, &session_id)?;

        self.runtime
            .resize_session(&PtySessionId::new(session.id.to_string()), cols, rows)
            .map_err(map_terminal_runtime_error)
    }
}

/// Handles explicit terminal kill requests without exposing PTY internals to adapters.
pub struct KillTerminalSessionHandler<SessionRepositoryPort, RuntimePort> {
    session_repository: SessionRepositoryPort,
    runtime: RuntimePort,
}

impl<SessionRepositoryPort, RuntimePort>
    KillTerminalSessionHandler<SessionRepositoryPort, RuntimePort>
{
    /// Builds the terminal kill handler from application-owned dependencies.
    pub fn new(session_repository: SessionRepositoryPort, runtime: RuntimePort) -> Self {
        Self {
            session_repository,
            runtime,
        }
    }
}

impl<SessionRepositoryPort, RuntimePort>
    KillTerminalSessionHandler<SessionRepositoryPort, RuntimePort>
where
    SessionRepositoryPort: SessionRepository,
    RuntimePort: TerminalRuntime,
{
    /// Requests explicit PTY termination for the running terminal session addressed by the caller.
    pub fn handle(&self, session_id: String) -> Result<(), ApplicationError> {
        let session = load_running_terminal_session(&self.session_repository, &session_id)?;

        self.runtime
            .kill_session(&PtySessionId::new(session.id.to_string()))
            .map_err(map_terminal_runtime_error)
    }
}

/// Handles PTY exit notifications so persisted session state stays consistent with runtime state.
pub struct HandleTerminalExitHandler<SessionRepositoryPort, ClockSource> {
    session_repository: SessionRepositoryPort,
    clock: ClockSource,
}

impl<SessionRepositoryPort, ClockSource>
    HandleTerminalExitHandler<SessionRepositoryPort, ClockSource>
{
    /// Builds the terminal-exit synchronization handler from application-owned dependencies.
    pub fn new(session_repository: SessionRepositoryPort, clock: ClockSource) -> Self {
        Self {
            session_repository,
            clock,
        }
    }
}

impl<SessionRepositoryPort, ClockSource>
    HandleTerminalExitHandler<SessionRepositoryPort, ClockSource>
where
    SessionRepositoryPort: SessionRepository,
    ClockSource: Clock,
{
    /// Persists the stopped terminal session state after a runtime exit event arrives.
    pub fn handle(&self, event: PtyLifecycleEvent) -> Result<(), ApplicationError> {
        match event {
            PtyLifecycleEvent::Exited { session_id, .. } => {
                let session_id = SessionId::new(session_id.to_string());
                let existing_session = self
                    .session_repository
                    .find_session(&session_id)
                    .map_err(|error| {
                        let error = ApplicationError::from_session_repository_error(error);
                        log_terminal_failure("handle_terminal_exit", Some(&session_id), &error);
                        error
                    })?
                    .ok_or_else(|| ApplicationError::SessionNotFound {
                        session_id: session_id.to_string(),
                    })?;
                let updated_session = DomainSession::new(
                    existing_session.id.clone(),
                    existing_session.task_id,
                    existing_session.agent_id,
                    existing_session.agent_session_id,
                    DomainSessionStatus::Stopped,
                    AuditFields::new(
                        existing_session.audit_fields.created_at,
                        self.clock.now_timestamp_millis(),
                        existing_session.audit_fields.is_deleted,
                    ),
                );

                self.session_repository
                    .update_session(updated_session)
                    .map_err(|error| {
                        let error = ApplicationError::from_session_repository_error(error);
                        log_terminal_failure("handle_terminal_exit", Some(&session_id), &error);
                        error
                    })?;
                log_terminal_success("handle_terminal_exit", &session_id);

                Ok(())
            }
        }
    }
}

/// Validates that the create-session request actually describes a terminal session startup.
fn validate_terminal_request(
    request: &CreateSessionRequest,
) -> Result<TerminalSessionStartup, ApplicationError> {
    if request.agent_id != AgentId::TERMINAL {
        return Err(ApplicationError::InvalidTerminalRequest {
            message: format!(
                "terminal session create requests must use agentId `{}`",
                AgentId::TERMINAL
            ),
        });
    }

    request
        .terminal
        .clone()
        .ok_or_else(|| ApplicationError::InvalidTerminalRequest {
            message: "terminal session create requests must include terminal startup dimensions"
                .to_string(),
        })
}

/// Verifies the stored worktree still belongs to the task being used for terminal startup.
fn validate_active_worktree<WorktreeRepositoryPort>(
    worktree_repository: &WorktreeRepositoryPort,
    task_id: &TaskId,
    worktree_id: &WorktreeId,
) -> Result<(), ApplicationError>
where
    WorktreeRepositoryPort: WorktreeRepository,
{
    let worktree = worktree_repository
        .find_worktree(worktree_id)
        .map_err(ApplicationError::from_worktree_repository_error)?
        .ok_or_else(|| ApplicationError::WorktreeNotFound {
            worktree_id: worktree_id.to_string(),
        })?;

    if worktree.task_id != *task_id {
        return Err(ApplicationError::WorktreeNotFound {
            worktree_id: worktree_id.to_string(),
        });
    }

    Ok(())
}

/// Loads one session and verifies it is both terminal-backed and still running.
fn load_running_terminal_session<SessionRepositoryPort>(
    session_repository: &SessionRepositoryPort,
    session_id: &str,
) -> Result<ora_domain::Session, ApplicationError>
where
    SessionRepositoryPort: SessionRepository,
{
    let session_id = SessionId::new(session_id);
    let session = session_repository
        .find_session(&session_id)
        .map_err(ApplicationError::from_session_repository_error)?
        .ok_or_else(|| ApplicationError::SessionNotFound {
            session_id: session_id.to_string(),
        })?;

    if session.agent_id != AgentId::terminal() {
        return Err(ApplicationError::TerminalSessionNotTerminal {
            session_id: session.id.to_string(),
        });
    }
    if session.status == DomainSessionStatus::Stopped {
        return Err(ApplicationError::TerminalSessionStopped {
            session_id: session.id.to_string(),
        });
    }

    Ok(session)
}

/// Translates the shared contract session status into the domain representation.
fn map_contract_session_status(status: SessionStatus) -> DomainSessionStatus {
    match status {
        SessionStatus::Running => DomainSessionStatus::Running,
        SessionStatus::Stopped => DomainSessionStatus::Stopped,
    }
}

/// Maps a domain terminal session into the shared public session contract shape.
fn map_session(session: DomainSession) -> ora_contracts::Session {
    ora_contracts::Session {
        id: session.id.to_string(),
        task_id: session.task_id.to_string(),
        agent_id: session.agent_id.to_string(),
        agent_session_id: session.agent_session_id,
        status: match session.status {
            DomainSessionStatus::Running => SessionStatus::Running,
            DomainSessionStatus::Stopped => SessionStatus::Stopped,
        },
    }
}

/// Derives the backend-owned worktree path for the addressed task.
fn worktree_path_for_task(work_dir: &Path, task_id: &TaskId) -> PathBuf {
    work_dir.join(task_id.to_string())
}

/// Converts runtime-owned failures into stable application terminal errors.
fn map_terminal_runtime_error(error: TerminalRuntimeError) -> ApplicationError {
    match error {
        TerminalRuntimeError::StartupFailed { message } => {
            ApplicationError::TerminalStartup { message }
        }
        TerminalRuntimeError::RuntimeMissing { session_id } => {
            ApplicationError::TerminalRuntimeMissing { session_id }
        }
        TerminalRuntimeError::AlreadyAttached { session_id } => {
            ApplicationError::TerminalAlreadyAttached { session_id }
        }
        TerminalRuntimeError::SessionStopped { session_id } => {
            ApplicationError::TerminalSessionStopped { session_id }
        }
        TerminalRuntimeError::ControlFailed { message } => {
            ApplicationError::TerminalStartup { message }
        }
    }
}

/// Emits the shared informational event shape for successful terminal lifecycle operations.
fn log_terminal_success(operation: &'static str, session_id: &SessionId) {
    ora_info!(
        message = "terminal operation completed",
        operation,
        session_id = session_id.to_string()
    );
}

/// Emits the shared error event shape for failed terminal lifecycle operations.
fn log_terminal_failure(
    operation: &'static str,
    session_id: Option<&SessionId>,
    error: &ApplicationError,
) {
    match session_id {
        Some(session_id) => {
            ora_error!(
                message = "terminal operation failed",
                operation,
                session_id = session_id.to_string(),
                error.kind = terminal_error_kind(error),
                error.message = error.to_string()
            );
        }
        None => {
            ora_error!(
                message = "terminal operation failed",
                operation,
                error.kind = terminal_error_kind(error),
                error.message = error.to_string()
            );
        }
    }
}

/// Converts terminal-oriented application errors into stable structured-log kind fields.
fn terminal_error_kind(error: &ApplicationError) -> &'static str {
    match error {
        ApplicationError::TerminalStartup { .. } => "terminal_startup",
        ApplicationError::TerminalRuntimeMissing { .. } => "terminal_runtime_missing",
        ApplicationError::TerminalAlreadyAttached { .. } => "terminal_already_attached",
        ApplicationError::TerminalSessionNotTerminal { .. } => "terminal_session_not_terminal",
        ApplicationError::TerminalSessionStopped { .. } => "terminal_session_stopped",
        ApplicationError::InvalidTerminalRequest { .. } => "invalid_terminal_request",
        ApplicationError::SessionNotFound { .. } => "session_not_found",
        ApplicationError::SessionRepository { .. } => "session_repository",
        ApplicationError::TaskNotFound { .. } => "task_not_found",
        ApplicationError::TaskRepository { .. } => "task_repository",
        ApplicationError::WorktreeNotFound { .. } => "worktree_not_found",
        ApplicationError::WorktreeRepository { .. } => "worktree_repository",
        ApplicationError::ProjectNotFound { .. }
        | ApplicationError::ProjectRepository { .. }
        | ApplicationError::ProjectOccupied { .. }
        | ApplicationError::ProjectWorkContextNotFound { .. }
        | ApplicationError::ProjectWorkContextRepository { .. }
        | ApplicationError::TaskWorktree { .. } => "terminal_other",
    }
}

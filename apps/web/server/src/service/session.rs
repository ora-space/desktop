use crate::bootstrap::SystemClock;
use ora_application::{
    ApplicationError, AttachTerminalSessionHandler, CreateSessionHandler,
    CreateTerminalSessionHandler, DeleteSessionHandler, GetSessionHandler,
    HandleTerminalExitHandler, KillTerminalSessionHandler, ListSessionsHandler,
    ResizeTerminalSessionHandler, SendTerminalInputHandler, TerminalAttachment, TerminalRuntime,
    TerminalRuntimeError, TerminalRuntimeRequest, TerminalRuntimeResult, TerminalStartupConfig,
    UpdateSessionHandler, UuidSessionIdGenerator,
};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    UpdateSessionRequest, UpdateSessionResponse,
};
use ora_db::{
    RepositoryPool, SqliteSessionRepository, SqliteTaskRepository, SqliteWorktreeRepository,
};
use ora_domain::AgentId;
use ora_pty::{
    PortablePtyProcessFactory, PtyRuntimeManager, PtyRuntimeManagerError, PtyServerToken,
    PtySessionControlError, PtySessionId, PtySessionStartRequest,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Groups the transport-facing session entry points for the web adapter.
pub struct SessionApi {
    create_session:
        CreateSessionHandler<SqliteSessionRepository, UuidSessionIdGenerator, SystemClock>,
    create_terminal_session: CreateTerminalSessionHandler<
        SqliteSessionRepository,
        SqliteTaskRepository,
        SqliteWorktreeRepository,
        WebTerminalRuntime,
        UuidSessionIdGenerator,
        SystemClock,
    >,
    get_session: GetSessionHandler<SqliteSessionRepository>,
    list_sessions: ListSessionsHandler<SqliteSessionRepository>,
    update_session: UpdateSessionHandler<SqliteSessionRepository, SystemClock>,
    delete_session: DeleteSessionHandler<SqliteSessionRepository, SystemClock>,
    attach_terminal_session:
        AttachTerminalSessionHandler<SqliteSessionRepository, WebTerminalRuntime>,
    send_terminal_input: SendTerminalInputHandler<SqliteSessionRepository, WebTerminalRuntime>,
    resize_terminal_session:
        ResizeTerminalSessionHandler<SqliteSessionRepository, WebTerminalRuntime>,
    kill_terminal_session: KillTerminalSessionHandler<SqliteSessionRepository, WebTerminalRuntime>,
    terminal_runtime: WebTerminalRuntime,
    terminal_server_token: PtyServerToken,
}

impl SessionApi {
    /// Builds the session transport API from the shared repository pool and clock source.
    pub fn new(pool: RepositoryPool, work_dir: PathBuf, clock: SystemClock) -> Self {
        let repository = SqliteSessionRepository::new(pool.clone());
        let terminal_server_token = PtyServerToken::new();
        let runtime = WebTerminalRuntime::new(Arc::new(PtyRuntimeManager::new(
            PortablePtyProcessFactory::new(),
            terminal_server_token.cancellation_token(),
            256 * 1024,
        )));
        let exit_handler = HandleTerminalExitHandler::new(repository.clone(), clock);
        let mut lifecycle_receiver = runtime.subscribe_lifecycle();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|error| panic!("expected terminal lifecycle runtime: {error}"));

            runtime.block_on(async move {
                loop {
                    match lifecycle_receiver.recv().await {
                        Ok(event) => {
                            let _ = exit_handler.handle(event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        });

        Self {
            create_session: CreateSessionHandler::new(
                repository.clone(),
                UuidSessionIdGenerator::new(),
                clock,
            ),
            create_terminal_session: CreateTerminalSessionHandler::new(
                repository.clone(),
                SqliteTaskRepository::new(pool.clone()),
                SqliteWorktreeRepository::new(pool.clone()),
                runtime.clone(),
                UuidSessionIdGenerator::new(),
                TerminalStartupConfig {
                    work_dir,
                    shell_program: default_shell_program(),
                },
                clock,
            ),
            get_session: GetSessionHandler::new(repository.clone()),
            list_sessions: ListSessionsHandler::new(repository.clone()),
            update_session: UpdateSessionHandler::new(repository.clone(), clock),
            delete_session: DeleteSessionHandler::new(repository.clone(), clock),
            attach_terminal_session: AttachTerminalSessionHandler::new(
                repository.clone(),
                runtime.clone(),
            ),
            send_terminal_input: SendTerminalInputHandler::new(repository.clone(), runtime.clone()),
            resize_terminal_session: ResizeTerminalSessionHandler::new(
                repository.clone(),
                runtime.clone(),
            ),
            kill_terminal_session: KillTerminalSessionHandler::new(repository, runtime.clone()),
            terminal_runtime: runtime,
            terminal_server_token,
        }
    }

    /// Accepts a create-session request and delegates the use case to the application layer.
    pub fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, ApplicationError> {
        if request.terminal.is_some() || request.agent_id == AgentId::TERMINAL {
            return self.create_terminal_session.handle(request);
        }

        self.create_session.handle(request)
    }

    /// Accepts a get-session request and delegates the use case to the application layer.
    pub fn get_session(
        &self,
        request: GetSessionRequest,
    ) -> Result<GetSessionResponse, ApplicationError> {
        self.get_session.handle(request)
    }

    /// Accepts a list-sessions request and delegates the use case to the application layer.
    pub fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, ApplicationError> {
        self.list_sessions.handle(request)
    }

    /// Accepts an update-session request and delegates the use case to the application layer.
    pub fn update_session(
        &self,
        request: UpdateSessionRequest,
    ) -> Result<UpdateSessionResponse, ApplicationError> {
        self.update_session.handle(request)
    }

    /// Accepts a delete-session request and delegates the use case to the application layer.
    pub fn delete_session(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, ApplicationError> {
        self.delete_session.handle(request)
    }

    /// Attaches one live client to the addressed running terminal session.
    pub fn attach_terminal_session(
        &self,
        session_id: String,
    ) -> Result<TerminalAttachment, ApplicationError> {
        self.attach_terminal_session.handle(session_id)
    }

    /// Detaches the currently attached client while keeping the PTY runtime alive.
    pub fn detach_terminal_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.terminal_runtime
            .detach_session(&PtySessionId::new(session_id))
            .map_err(map_terminal_runtime_error)
    }

    /// Sends raw input to the addressed running terminal session.
    pub fn send_terminal_input(
        &self,
        session_id: String,
        data: String,
    ) -> Result<(), ApplicationError> {
        self.send_terminal_input.handle(session_id, data)
    }

    /// Applies a resize request to the addressed running terminal session.
    pub fn resize_terminal_session(
        &self,
        session_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), ApplicationError> {
        self.resize_terminal_session.handle(session_id, cols, rows)
    }

    /// Requests explicit PTY termination for the addressed running terminal session.
    pub fn kill_terminal_session(&self, session_id: String) -> Result<(), ApplicationError> {
        self.kill_terminal_session.handle(session_id)
    }

    /// Cancels the root terminal server token so every active session begins shutdown.
    pub fn shutdown_terminals(&self) {
        self.terminal_server_token.cancel();
    }
}

/// Wraps the shared PTY runtime manager in a local adapter type the web server can implement ports for.
#[derive(Clone)]
struct WebTerminalRuntime {
    manager: Arc<PtyRuntimeManager<PortablePtyProcessFactory>>,
}

impl WebTerminalRuntime {
    /// Builds the local terminal runtime adapter around the shared PTY runtime manager.
    fn new(manager: Arc<PtyRuntimeManager<PortablePtyProcessFactory>>) -> Self {
        Self { manager }
    }

    /// Returns a lifecycle receiver so the session API can persist exit-driven state changes.
    fn subscribe_lifecycle(&self) -> tokio::sync::broadcast::Receiver<ora_pty::PtyLifecycleEvent> {
        self.manager.subscribe_lifecycle()
    }
}

impl TerminalRuntime for WebTerminalRuntime {
    /// Starts one PTY runtime session using the shared runtime manager.
    fn start_session(
        &self,
        request: TerminalRuntimeRequest,
    ) -> Result<TerminalRuntimeResult, TerminalRuntimeError> {
        self.manager
            .start_session(PtySessionStartRequest {
                session_id: request.session_id.clone(),
                cwd: request.cwd,
                program: request.program,
                args: request.args,
                cols: request.cols,
                rows: request.rows,
            })
            .map(|handle| TerminalRuntimeResult {
                session_id: handle.session_id,
            })
            .map_err(|error| match error {
                PtyRuntimeManagerError::SessionAlreadyExists { session_id } => {
                    TerminalRuntimeError::ControlFailed {
                        message: format!("pty session already exists: {session_id}"),
                    }
                }
                PtyRuntimeManagerError::Spawn(error) => TerminalRuntimeError::StartupFailed {
                    message: error.to_string(),
                },
            })
    }

    /// Attaches one live client and returns replay plus a live event receiver.
    fn attach_session(
        &self,
        session_id: &PtySessionId,
    ) -> Result<TerminalAttachment, TerminalRuntimeError> {
        self.manager
            .attach_session(session_id)
            .map(TerminalAttachment::from_pty_attachment)
            .map_err(map_ptysession_control_error)
    }

    /// Detaches one live client while keeping the PTY runtime available for reconnect.
    fn detach_session(&self, session_id: &PtySessionId) -> Result<(), TerminalRuntimeError> {
        self.manager
            .detach_session(session_id)
            .map_err(map_ptysession_control_error)
    }

    /// Sends raw terminal input into the addressed PTY runtime.
    fn send_input(
        &self,
        session_id: &PtySessionId,
        data: String,
    ) -> Result<(), TerminalRuntimeError> {
        self.manager
            .send_input(session_id, data)
            .map_err(map_ptysession_control_error)
    }

    /// Applies a resize request to the addressed PTY runtime.
    fn resize_session(
        &self,
        session_id: &PtySessionId,
        cols: u16,
        rows: u16,
    ) -> Result<(), TerminalRuntimeError> {
        self.manager
            .resize_session(session_id, cols, rows)
            .map_err(map_ptysession_control_error)
    }

    /// Requests explicit PTY termination for the addressed runtime.
    fn kill_session(&self, session_id: &PtySessionId) -> Result<(), TerminalRuntimeError> {
        self.manager
            .kill_session(session_id)
            .map_err(map_ptysession_control_error)
    }
}

/// Translates PTY runtime control errors into stable application terminal runtime errors.
fn map_ptysession_control_error(error: PtySessionControlError) -> TerminalRuntimeError {
    match error {
        PtySessionControlError::SessionMissing { session_id } => {
            TerminalRuntimeError::RuntimeMissing {
                session_id: session_id.to_string(),
            }
        }
        PtySessionControlError::SessionExited { session_id } => {
            TerminalRuntimeError::SessionStopped {
                session_id: session_id.to_string(),
            }
        }
        PtySessionControlError::AlreadyAttached { session_id } => {
            TerminalRuntimeError::AlreadyAttached {
                session_id: session_id.to_string(),
            }
        }
        PtySessionControlError::NotAttached { session_id } => {
            TerminalRuntimeError::RuntimeMissing {
                session_id: session_id.to_string(),
            }
        }
        PtySessionControlError::ControlFailed { message } => {
            TerminalRuntimeError::ControlFailed { message }
        }
    }
}

/// Translates terminal runtime errors into stable application errors for adapter-managed cleanup.
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

/// Returns the shell program used for first-slice task terminal sessions.
fn default_shell_program() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

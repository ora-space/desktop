use super::handlers::{
    AttachTerminalSessionHandler, CreateTerminalSessionHandler, HandleTerminalExitHandler,
};
use super::ports::{
    TerminalAttachment, TerminalRuntime, TerminalRuntimeError, TerminalRuntimeRequest,
    TerminalRuntimeResult, TerminalStartupConfig,
};
use crate::session::{SessionIdGenerator, SessionRepository, SessionRepositoryError};
use crate::task::{TaskRepository, TaskRepositoryError};
use crate::worktree::{WorktreeRepository, WorktreeRepositoryError};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, SessionStatus, TerminalSessionStartup,
};
use ora_domain::{
    AgentId, AuditFields, ProjectId, Session, SessionId, SessionStatus as DomainSessionStatus,
    Task, TaskId, TaskStatus, Worktree, WorktreeActivity, WorktreeId,
};
use ora_pty::{PtyLifecycleEvent, PtySessionAttachment, PtySessionId, PtySessionReplay};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::path::PathBuf;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// Verifies terminal startup persists the session and launches the PTY runtime for the task worktree.
#[test]
fn creates_terminal_sessions() {
    let session_repository = FakeSessionRepository::default();
    let task_repository = FakeTaskRepository::with_task(Task::new(
        TaskId::new("task-1"),
        ProjectId::new("project-1"),
        "Terminal task",
        TaskStatus::Doing,
        Some(WorktreeId::new("worktree-1")),
        AuditFields::new(10, 10, false),
    ));
    let worktree_repository = FakeWorktreeRepository::with_worktree(Worktree::new(
        WorktreeId::new("worktree-1"),
        TaskId::new("task-1"),
        Some("task/task-1".to_string()),
        WorktreeActivity::Active,
        AuditFields::new(10, 10, false),
    ));
    let runtime = FakeTerminalRuntime::default();
    let handler = CreateTerminalSessionHandler::new(
        session_repository.clone(),
        task_repository,
        worktree_repository,
        runtime.clone(),
        FixedSessionIdGenerator::new("session-1"),
        TerminalStartupConfig {
            work_dir: PathBuf::from("/tmp/worktrees"),
            shell_program: "/bin/bash".to_string(),
        },
        FixedClock::new(100),
    );

    let response = handler.handle(CreateSessionRequest {
        task_id: "task-1".to_string(),
        agent_id: "terminal".to_string(),
        agent_session_id: None,
        status: SessionStatus::Running,
        terminal: Some(TerminalSessionStartup {
            cols: 100,
            rows: 40,
        }),
    });

    assert_eq!(
        response,
        Ok(CreateSessionResponse {
            session: ora_contracts::Session {
                id: "session-1".to_string(),
                task_id: "task-1".to_string(),
                agent_id: "terminal".to_string(),
                agent_session_id: None,
                status: SessionStatus::Running,
            },
        })
    );
    assert_eq!(
        runtime.requests(),
        vec![TerminalRuntimeRequest {
            session_id: PtySessionId::new("session-1"),
            cwd: PathBuf::from("/tmp/worktrees/task-1"),
            program: "/bin/bash".to_string(),
            args: Vec::new(),
            cols: 100,
            rows: 40,
        }]
    );
    assert_eq!(
        session_repository.visible_sessions(),
        vec![Session::new(
            SessionId::new("session-1"),
            TaskId::new("task-1"),
            AgentId::terminal(),
            None,
            DomainSessionStatus::Running,
            AuditFields::new(100, 100, false),
        )]
    );
}

/// Verifies startup failures leave the persisted session visible but stopped for diagnostics.
#[test]
fn marks_terminal_sessions_stopped_when_runtime_startup_fails() {
    let session_repository = FakeSessionRepository::default();
    let task_repository = FakeTaskRepository::with_task(Task::new(
        TaskId::new("task-1"),
        ProjectId::new("project-1"),
        "Terminal task",
        TaskStatus::Doing,
        Some(WorktreeId::new("worktree-1")),
        AuditFields::new(10, 10, false),
    ));
    let worktree_repository = FakeWorktreeRepository::with_worktree(Worktree::new(
        WorktreeId::new("worktree-1"),
        TaskId::new("task-1"),
        Some("task/task-1".to_string()),
        WorktreeActivity::Active,
        AuditFields::new(10, 10, false),
    ));
    let runtime = FakeTerminalRuntime::failing_startup("spawn failed");
    let handler = CreateTerminalSessionHandler::new(
        session_repository.clone(),
        task_repository,
        worktree_repository,
        runtime,
        FixedSessionIdGenerator::new("session-1"),
        TerminalStartupConfig {
            work_dir: PathBuf::from("/tmp/worktrees"),
            shell_program: "/bin/bash".to_string(),
        },
        FixedClock::new(100),
    );

    let error = handler.handle(CreateSessionRequest {
        task_id: "task-1".to_string(),
        agent_id: "terminal".to_string(),
        agent_session_id: None,
        status: SessionStatus::Running,
        terminal: Some(TerminalSessionStartup { cols: 80, rows: 24 }),
    });

    assert_eq!(
        error,
        Err(ApplicationError::TerminalStartup {
            message: "spawn failed".to_string(),
        })
    );
    assert_eq!(
        session_repository.visible_sessions(),
        vec![Session::new(
            SessionId::new("session-1"),
            TaskId::new("task-1"),
            AgentId::terminal(),
            None,
            DomainSessionStatus::Stopped,
            AuditFields::new(100, 100, false),
        )]
    );
}

/// Verifies duplicate live attachment returns a stable terminal-specific application error.
#[test]
fn rejects_duplicate_terminal_attachment() {
    let session_repository = FakeSessionRepository::with_sessions(vec![Session::new(
        SessionId::new("session-1"),
        TaskId::new("task-1"),
        AgentId::terminal(),
        None,
        DomainSessionStatus::Running,
        AuditFields::new(10, 10, false),
    )]);
    let runtime = FakeTerminalRuntime::already_attached("session-1");
    let handler = AttachTerminalSessionHandler::new(session_repository, runtime);

    assert_eq!(
        handler.handle("session-1".to_string()).map(|_| ()),
        Err(ApplicationError::TerminalAlreadyAttached {
            session_id: "session-1".to_string(),
        })
    );
}

/// Verifies PTY exit synchronization persists the terminal session as stopped.
#[test]
fn persists_stopped_status_on_terminal_exit() {
    let session_repository = FakeSessionRepository::with_sessions(vec![Session::new(
        SessionId::new("session-1"),
        TaskId::new("task-1"),
        AgentId::terminal(),
        None,
        DomainSessionStatus::Running,
        AuditFields::new(10, 10, false),
    )]);
    let handler = HandleTerminalExitHandler::new(session_repository.clone(), FixedClock::new(200));

    assert_eq!(
        handler.handle(PtyLifecycleEvent::Exited {
            session_id: PtySessionId::new("session-1"),
            exit_code: Some(0),
        }),
        Ok(())
    );
    assert_eq!(
        session_repository.visible_sessions(),
        vec![Session::new(
            SessionId::new("session-1"),
            TaskId::new("task-1"),
            AgentId::terminal(),
            None,
            DomainSessionStatus::Stopped,
            AuditFields::new(10, 200, false),
        )]
    );
}

#[derive(Clone, Default)]
struct FakeSessionRepository {
    sessions: std::rc::Rc<RefCell<Vec<Session>>>,
}

impl FakeSessionRepository {
    /// Builds a fake session repository with the provided session rows.
    fn with_sessions(sessions: Vec<Session>) -> Self {
        Self {
            sessions: std::rc::Rc::new(RefCell::new(sessions)),
        }
    }

    /// Returns every visible stored session for deep state assertions.
    fn visible_sessions(&self) -> Vec<Session> {
        self.sessions.borrow().clone()
    }
}

impl SessionRepository for FakeSessionRepository {
    /// Persists a new session row in memory.
    fn create_session(&self, session: Session) -> Result<Session, SessionRepositoryError> {
        self.sessions.borrow_mut().push(session.clone());
        Ok(session)
    }

    /// Loads one visible session row by identifier from memory.
    fn find_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        Ok(self
            .sessions
            .borrow()
            .iter()
            .find(|session| session.id == *session_id)
            .cloned())
    }

    /// Returns every stored session row in insertion order.
    fn list_sessions(&self) -> Result<Vec<Session>, SessionRepositoryError> {
        Ok(self.sessions.borrow().clone())
    }

    /// Replaces one stored session row in memory.
    fn update_session(&self, session: Session) -> Result<Session, SessionRepositoryError> {
        let mut sessions = self.sessions.borrow_mut();
        let existing_session = sessions
            .iter_mut()
            .find(|existing_session| existing_session.id == session.id)
            .ok_or_else(|| {
                SessionRepositoryError::OperationFailed("missing session".to_string())
            })?;

        *existing_session = session.clone();

        Ok(session)
    }

    /// Marks a stored session deleted for compatibility with the shared repository trait.
    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, SessionRepositoryError> {
        let mut sessions = self.sessions.borrow_mut();
        if let Some(session) = sessions
            .iter_mut()
            .find(|session| session.id == *session_id)
        {
            session.audit_fields.updated_at = deleted_at;
            session.audit_fields.is_deleted = true;
            return Ok(true);
        }

        Ok(false)
    }
}

struct FakeTaskRepository {
    task: Task,
}

impl FakeTaskRepository {
    /// Builds a fake task repository around one stored task.
    fn with_task(task: Task) -> Self {
        Self { task }
    }
}

impl TaskRepository for FakeTaskRepository {
    /// Rejects task creation because these tests only need lookup behavior.
    fn create_task(&self, _task: Task) -> Result<Task, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed("unused".to_string()))
    }

    /// Returns the stored task when the identifier matches.
    fn find_task(&self, task_id: &TaskId) -> Result<Option<Task>, TaskRepositoryError> {
        Ok((self.task.id == *task_id).then(|| self.task.clone()))
    }

    /// Returns the stored task in list form for trait completeness.
    fn list_tasks(&self) -> Result<Vec<Task>, TaskRepositoryError> {
        Ok(vec![self.task.clone()])
    }

    /// Rejects task updates because these tests only need lookup behavior.
    fn update_task(&self, _task: Task) -> Result<Task, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed("unused".to_string()))
    }

    /// Rejects task deletion because these tests only need lookup behavior.
    fn soft_delete_task(
        &self,
        _task_id: &TaskId,
        _deleted_at: i64,
    ) -> Result<bool, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed("unused".to_string()))
    }
}

struct FakeWorktreeRepository {
    worktree: Worktree,
}

impl FakeWorktreeRepository {
    /// Builds a fake worktree repository around one stored worktree.
    fn with_worktree(worktree: Worktree) -> Self {
        Self { worktree }
    }
}

impl WorktreeRepository for FakeWorktreeRepository {
    /// Rejects worktree creation because these tests only need lookup behavior.
    fn create_worktree(&self, _worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "unused".to_string(),
        ))
    }

    /// Returns the stored worktree when the identifier matches.
    fn find_worktree(
        &self,
        worktree_id: &WorktreeId,
    ) -> Result<Option<Worktree>, WorktreeRepositoryError> {
        Ok((self.worktree.id == *worktree_id).then(|| self.worktree.clone()))
    }

    /// Returns the stored worktree in list form for trait completeness.
    fn list_worktrees(&self) -> Result<Vec<Worktree>, WorktreeRepositoryError> {
        Ok(vec![self.worktree.clone()])
    }

    /// Rejects worktree updates because these tests only need lookup behavior.
    fn update_worktree(&self, _worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "unused".to_string(),
        ))
    }

    /// Rejects worktree deletion because these tests only need lookup behavior.
    fn soft_delete_worktree(
        &self,
        _worktree_id: &WorktreeId,
        _deleted_at: i64,
    ) -> Result<bool, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "unused".to_string(),
        ))
    }
}

#[derive(Clone)]
struct FakeTerminalRuntime {
    requests: std::rc::Rc<RefCell<Vec<TerminalRuntimeRequest>>>,
    startup_error: Option<TerminalRuntimeError>,
    attach_error: Option<TerminalRuntimeError>,
}

impl Default for FakeTerminalRuntime {
    /// Builds a successful fake terminal runtime with empty captured state.
    fn default() -> Self {
        Self {
            requests: std::rc::Rc::new(RefCell::new(Vec::new())),
            startup_error: None,
            attach_error: None,
        }
    }
}

impl FakeTerminalRuntime {
    /// Builds a fake runtime that fails terminal startup with the provided message.
    fn failing_startup(message: &str) -> Self {
        Self {
            startup_error: Some(TerminalRuntimeError::StartupFailed {
                message: message.to_string(),
            }),
            ..Self::default()
        }
    }

    /// Builds a fake runtime that reports the addressed session already attached.
    fn already_attached(session_id: &str) -> Self {
        Self {
            attach_error: Some(TerminalRuntimeError::AlreadyAttached {
                session_id: session_id.to_string(),
            }),
            ..Self::default()
        }
    }

    /// Returns the captured startup requests for deep assertions.
    fn requests(&self) -> Vec<TerminalRuntimeRequest> {
        self.requests.borrow().clone()
    }
}

impl TerminalRuntime for FakeTerminalRuntime {
    /// Captures startup requests or returns the configured fake startup error.
    fn start_session(
        &self,
        request: TerminalRuntimeRequest,
    ) -> Result<TerminalRuntimeResult, TerminalRuntimeError> {
        if let Some(error) = self.startup_error.clone() {
            return Err(error);
        }

        self.requests.borrow_mut().push(request.clone());

        Ok(TerminalRuntimeResult {
            session_id: request.session_id,
        })
    }

    /// Returns a no-op attachment or the configured fake attach error.
    fn attach_session(
        &self,
        _session_id: &PtySessionId,
    ) -> Result<TerminalAttachment, TerminalRuntimeError> {
        if let Some(error) = self.attach_error.clone() {
            return Err(error);
        }

        let (_sender, receiver) = broadcast::channel(4);
        Ok(TerminalAttachment::from_pty_attachment(
            PtySessionAttachment {
                state: ora_pty::PtyClientAttachmentState::Attached,
                replay: PtySessionReplay {
                    chunks: vec![ora_pty::PtyOutputChunk {
                        data: "hello\n".to_string(),
                    }],
                },
                session_token: CancellationToken::new(),
                output_receiver: receiver,
            },
        ))
    }

    /// Ignores detach requests for these focused unit tests.
    fn detach_session(&self, _session_id: &PtySessionId) -> Result<(), TerminalRuntimeError> {
        Ok(())
    }

    /// Ignores input requests for these focused unit tests.
    fn send_input(
        &self,
        _session_id: &PtySessionId,
        _data: String,
    ) -> Result<(), TerminalRuntimeError> {
        Ok(())
    }

    /// Ignores resize requests for these focused unit tests.
    fn resize_session(
        &self,
        _session_id: &PtySessionId,
        _cols: u16,
        _rows: u16,
    ) -> Result<(), TerminalRuntimeError> {
        Ok(())
    }

    /// Ignores kill requests for these focused unit tests.
    fn kill_session(&self, _session_id: &PtySessionId) -> Result<(), TerminalRuntimeError> {
        Ok(())
    }
}

struct FixedSessionIdGenerator {
    session_id: SessionId,
}

impl FixedSessionIdGenerator {
    /// Builds a fixed session identifier generator for deterministic tests.
    fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: SessionId::new(session_id),
        }
    }
}

impl SessionIdGenerator for FixedSessionIdGenerator {
    /// Returns the same fixed session identifier on every invocation.
    fn generate_session_id(&self) -> SessionId {
        self.session_id.clone()
    }
}

#[derive(Clone, Copy)]
struct FixedClock {
    now: i64,
}

impl FixedClock {
    /// Builds a fixed clock for deterministic audit-field assertions.
    fn new(now: i64) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    /// Returns the fixed timestamp configured for the test.
    fn now_timestamp_millis(&self) -> i64 {
        self.now
    }
}

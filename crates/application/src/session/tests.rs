use crate::{
    ApplicationError, Clock, CreateSessionHandler, DeleteSessionHandler, GetSessionHandler,
    ListSessionsHandler, SessionIdGenerator, SessionRepository, SessionRepositoryError,
    UpdateSessionHandler,
};
use ora_contracts::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse,
    Session as ContractSession, SessionStatus as ContractSessionStatus, UpdateSessionRequest,
    UpdateSessionResponse,
};
use ora_domain::{
    AgentId, AuditFields, Session, SessionId, SessionStatus as DomainSessionStatus, TaskId,
};
use ora_logging::{with_recorded_trace_logging, with_trace_logging};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Verifies create handlers build domain sessions and return the shared contract response.
#[test]
fn creates_sessions_with_generated_identity_and_clock_values() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeSessionRepository::default());
        let handler = CreateSessionHandler::new(
            repository.clone(),
            FixedSessionIdGenerator::new("session-1"),
            FixedClock::new(1_700_000_000_000),
        );

        let response = match handler.handle(CreateSessionRequest {
            task_id: "task-1".to_string(),
            agent_id: "agent-1".to_string(),
            agent_session_id: Some("provider-1".to_string()),
            status: ContractSessionStatus::Running,
        }) {
            Ok(response) => response,
            Err(error) => panic!("create handler failed: {error}"),
        };

        assert_eq!(
            response,
            CreateSessionResponse {
                session: ContractSession {
                    id: "session-1".to_string(),
                    task_id: "task-1".to_string(),
                    agent_id: "agent-1".to_string(),
                    agent_session_id: Some("provider-1".to_string()),
                    status: ContractSessionStatus::Running,
                },
            }
        );
        assert_eq!(
            repository.visible_sessions(),
            vec![Session::new(
                SessionId::new("session-1"),
                TaskId::new("task-1"),
                AgentId::new("agent-1"),
                Some("provider-1".to_string()),
                DomainSessionStatus::Running,
                AuditFields::new(1_700_000_000_000, 1_700_000_000_000, false),
            )]
        );
    });
}

/// Verifies get handlers return the shared contract projection for existing sessions.
#[test]
fn gets_sessions_by_identifier() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeSessionRepository::with_sessions(vec![Session::new(
            SessionId::new("session-1"),
            TaskId::new("task-1"),
            AgentId::new("agent-1"),
            None,
            DomainSessionStatus::Stopped,
            AuditFields::new(1, 2, false),
        )]));
        let handler = GetSessionHandler::new(repository);

        let response = match handler.handle(GetSessionRequest {
            session_id: "session-1".to_string(),
        }) {
            Ok(response) => response,
            Err(error) => panic!("get handler failed: {error}"),
        };

        assert_eq!(
            response,
            GetSessionResponse {
                session: ContractSession {
                    id: "session-1".to_string(),
                    task_id: "task-1".to_string(),
                    agent_id: "agent-1".to_string(),
                    agent_session_id: None,
                    status: ContractSessionStatus::Stopped,
                },
            }
        );
    });
}

/// Verifies list handlers map every stored session into the shared contract payload.
#[test]
fn lists_visible_sessions() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeSessionRepository::with_sessions(vec![
            Session::new(
                SessionId::new("session-1"),
                TaskId::new("task-1"),
                AgentId::new("agent-1"),
                None,
                DomainSessionStatus::Stopped,
                AuditFields::new(1, 2, false),
            ),
            Session::new(
                SessionId::new("session-2"),
                TaskId::new("task-2"),
                AgentId::new("agent-2"),
                Some("provider-2".to_string()),
                DomainSessionStatus::Running,
                AuditFields::new(3, 4, false),
            ),
        ]));
        let handler = ListSessionsHandler::new(repository);

        let response = match handler.handle(ListSessionsRequest {}) {
            Ok(response) => response,
            Err(error) => panic!("list handler failed: {error}"),
        };

        assert_eq!(
            response,
            ListSessionsResponse {
                sessions: vec![
                    ContractSession {
                        id: "session-1".to_string(),
                        task_id: "task-1".to_string(),
                        agent_id: "agent-1".to_string(),
                        agent_session_id: None,
                        status: ContractSessionStatus::Stopped,
                    },
                    ContractSession {
                        id: "session-2".to_string(),
                        task_id: "task-2".to_string(),
                        agent_id: "agent-2".to_string(),
                        agent_session_id: Some("provider-2".to_string()),
                        status: ContractSessionStatus::Running,
                    },
                ],
            }
        );
    });
}

/// Verifies update handlers preserve created timestamps while refreshing mutable fields.
#[test]
fn updates_sessions_with_refreshed_timestamps() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeSessionRepository::with_sessions(vec![Session::new(
            SessionId::new("session-1"),
            TaskId::new("task-1"),
            AgentId::new("agent-1"),
            None,
            DomainSessionStatus::Stopped,
            AuditFields::new(10, 20, false),
        )]));
        let handler = UpdateSessionHandler::new(repository.clone(), FixedClock::new(30));

        let response = match handler.handle(UpdateSessionRequest {
            session_id: "session-1".to_string(),
            task_id: "task-2".to_string(),
            agent_id: "agent-2".to_string(),
            agent_session_id: Some("provider-2".to_string()),
            status: ContractSessionStatus::Running,
        }) {
            Ok(response) => response,
            Err(error) => panic!("update handler failed: {error}"),
        };

        assert_eq!(
            response,
            UpdateSessionResponse {
                session: ContractSession {
                    id: "session-1".to_string(),
                    task_id: "task-2".to_string(),
                    agent_id: "agent-2".to_string(),
                    agent_session_id: Some("provider-2".to_string()),
                    status: ContractSessionStatus::Running,
                },
            }
        );
        assert_eq!(
            repository.visible_sessions(),
            vec![Session::new(
                SessionId::new("session-1"),
                TaskId::new("task-2"),
                AgentId::new("agent-2"),
                Some("provider-2".to_string()),
                DomainSessionStatus::Running,
                AuditFields::new(10, 30, false),
            )]
        );
    });
}

/// Verifies delete handlers keep the external CRUD contract while soft-deleting storage state.
#[test]
fn deletes_sessions_through_soft_delete_repository_calls() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeSessionRepository::with_sessions(vec![Session::new(
            SessionId::new("session-1"),
            TaskId::new("task-1"),
            AgentId::new("agent-1"),
            None,
            DomainSessionStatus::Stopped,
            AuditFields::new(10, 20, false),
        )]));
        let handler = DeleteSessionHandler::new(repository.clone(), FixedClock::new(40));

        let response = match handler.handle(DeleteSessionRequest {
            session_id: "session-1".to_string(),
        }) {
            Ok(response) => response,
            Err(error) => panic!("delete handler failed: {error}"),
        };

        assert_eq!(
            response,
            DeleteSessionResponse {
                session_id: "session-1".to_string(),
            }
        );
        assert_eq!(repository.visible_sessions(), Vec::<Session>::new());
        assert_eq!(
            repository.all_sessions(),
            vec![Session::new(
                SessionId::new("session-1"),
                TaskId::new("task-1"),
                AgentId::new("agent-1"),
                None,
                DomainSessionStatus::Stopped,
                AuditFields::new(10, 40, true),
            )]
        );
    });
}

/// Verifies handlers expose stable application errors for missing sessions and repository failures.
#[test]
fn reports_application_errors() {
    with_trace_logging(|| {
        let missing_repository = Rc::new(FakeSessionRepository::default());
        let get_handler = GetSessionHandler::new(missing_repository);
        let failing_repository = Rc::new(FakeSessionRepository::default());
        failing_repository.fail_next(SessionRepositoryError::OperationFailed(
            "storage unavailable".to_string(),
        ));
        let list_handler = ListSessionsHandler::new(failing_repository);

        let missing_error = match get_handler.handle(GetSessionRequest {
            session_id: "missing".to_string(),
        }) {
            Ok(response) => panic!("expected missing error, got response: {response:?}"),
            Err(error) => error,
        };
        let repository_error = match list_handler.handle(ListSessionsRequest {}) {
            Ok(response) => panic!("expected repository error, got response: {response:?}"),
            Err(error) => error,
        };

        assert_eq!(
            missing_error,
            ApplicationError::SessionNotFound {
                session_id: "missing".to_string(),
            }
        );
        assert_eq!(
            repository_error,
            ApplicationError::SessionRepository {
                message: "storage unavailable".to_string(),
            }
        );
    });
}

/// Verifies session handlers emit structured success and failure events under a scoped subscriber.
#[test]
fn emits_structured_operational_events() {
    let recorder = EventRecorder::default();
    with_recorded_trace_logging(recorder.layer(), || {
        let create_repository = Rc::new(FakeSessionRepository::default());
        let create_handler = CreateSessionHandler::new(
            create_repository,
            FixedSessionIdGenerator::new("session-42"),
            FixedClock::new(5),
        );
        let get_handler = GetSessionHandler::new(Rc::new(FakeSessionRepository::default()));

        create_handler
            .handle(CreateSessionRequest {
                task_id: "task-1".to_string(),
                agent_id: "agent-1".to_string(),
                agent_session_id: None,
                status: ContractSessionStatus::Stopped,
            })
            .unwrap();
        assert_eq!(
            get_handler
                .handle(GetSessionRequest {
                    session_id: "missing".to_string(),
                })
                .unwrap_err(),
            ApplicationError::SessionNotFound {
                session_id: "missing".to_string(),
            }
        );
    });

    assert_eq!(
        recorder.events(),
        vec![
            LoggedEvent {
                level: "INFO".to_string(),
                target: "ora_application::session::handlers".to_string(),
                fields: BTreeMap::from([
                    (
                        "message".to_string(),
                        "session operation completed".to_string(),
                    ),
                    ("method".to_string(), "log_session_success".to_string()),
                    ("operation".to_string(), "create_session".to_string()),
                    ("session_id".to_string(), "session-42".to_string()),
                ]),
            },
            LoggedEvent {
                level: "ERROR".to_string(),
                target: "ora_application::session::handlers".to_string(),
                fields: BTreeMap::from([
                    ("error.kind".to_string(), "session_not_found".to_string()),
                    (
                        "error.message".to_string(),
                        "session not found: missing".to_string(),
                    ),
                    (
                        "message".to_string(),
                        "session operation failed".to_string()
                    ),
                    ("method".to_string(), "log_session_failure".to_string()),
                    ("operation".to_string(), "get_session".to_string()),
                    ("session_id".to_string(), "missing".to_string()),
                ]),
            },
        ]
    );
}

#[derive(Debug, Default)]
struct FakeSessionRepository {
    sessions: RefCell<Vec<Session>>,
    next_error: RefCell<Option<SessionRepositoryError>>,
}

impl FakeSessionRepository {
    /// Builds a fake repository seeded with the provided session rows.
    fn with_sessions(sessions: Vec<Session>) -> Self {
        Self {
            sessions: RefCell::new(sessions),
            next_error: RefCell::new(None),
        }
    }

    /// Configures the next repository call to fail with a deterministic error.
    fn fail_next(&self, error: SessionRepositoryError) {
        self.next_error.replace(Some(error));
    }

    /// Returns every non-deleted session so tests can assert visible repository state.
    fn visible_sessions(&self) -> Vec<Session> {
        self.sessions
            .borrow()
            .iter()
            .filter(|session| !session.audit_fields.is_deleted)
            .cloned()
            .collect()
    }

    /// Returns all stored sessions, including soft-deleted rows, for state assertions.
    fn all_sessions(&self) -> Vec<Session> {
        self.sessions.borrow().clone()
    }

    /// Returns a queued error when a test wants to simulate repository failure.
    fn take_error(&self) -> Result<(), SessionRepositoryError> {
        match self.next_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl SessionRepository for Rc<FakeSessionRepository> {
    fn create_session(&self, session: Session) -> Result<Session, SessionRepositoryError> {
        self.take_error()?;

        self.sessions.borrow_mut().push(session.clone());
        Ok(session)
    }

    fn find_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        self.take_error()?;

        Ok(self
            .sessions
            .borrow()
            .iter()
            .find(|session| session.id == *session_id && !session.audit_fields.is_deleted)
            .cloned())
    }

    fn list_sessions(&self) -> Result<Vec<Session>, SessionRepositoryError> {
        self.take_error()?;

        Ok(self.visible_sessions())
    }

    fn update_session(&self, session: Session) -> Result<Session, SessionRepositoryError> {
        self.take_error()?;

        let mut sessions = self.sessions.borrow_mut();
        if let Some(existing_session) = sessions.iter_mut().find(|existing_session| {
            existing_session.id == session.id && !existing_session.audit_fields.is_deleted
        }) {
            *existing_session = session.clone();
            Ok(session)
        } else {
            Err(SessionRepositoryError::OperationFailed(format!(
                "missing session during update: {}",
                session.id
            )))
        }
    }

    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, SessionRepositoryError> {
        self.take_error()?;

        let mut sessions = self.sessions.borrow_mut();
        if let Some(session) = sessions
            .iter_mut()
            .find(|session| session.id == *session_id && !session.audit_fields.is_deleted)
        {
            session.audit_fields.updated_at = deleted_at;
            session.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

struct FixedSessionIdGenerator {
    session_id: SessionId,
}

impl FixedSessionIdGenerator {
    /// Builds an identifier generator that always returns the provided session id.
    fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: SessionId::new(session_id),
        }
    }
}

impl SessionIdGenerator for FixedSessionIdGenerator {
    fn generate_session_id(&self) -> SessionId {
        self.session_id.clone()
    }
}

struct FixedClock {
    timestamp_millis: i64,
}

impl FixedClock {
    /// Builds a clock that always returns the provided timestamp.
    fn new(timestamp_millis: i64) -> Self {
        Self { timestamp_millis }
    }
}

impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.timestamp_millis
    }
}

/// Captures one emitted event in a comparison-friendly structure for logging assertions.
#[derive(Clone, Debug, Eq, PartialEq)]
struct LoggedEvent {
    level: String,
    target: String,
    fields: BTreeMap<String, String>,
}

/// Records tracing events into shared memory so tests can assert full structured outcomes.
#[derive(Clone, Debug, Default)]
struct EventRecorder {
    events: Arc<Mutex<Vec<LoggedEvent>>>,
}

impl EventRecorder {
    /// Builds the recording layer attached to one scoped test subscriber.
    fn layer(&self) -> RecordingLayer {
        RecordingLayer {
            events: self.events.clone(),
        }
    }

    /// Returns every captured event in emission order.
    fn events(&self) -> Vec<LoggedEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// Pushes each tracing event into the shared recorder without relying on global subscriber state.
#[derive(Clone, Debug)]
struct RecordingLayer {
    events: Arc<Mutex<Vec<LoggedEvent>>>,
}

impl<S> Layer<S> for RecordingLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    /// Converts each event into a stable, fully comparable structure for test assertions.
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(LoggedEvent {
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            fields: visitor.fields,
        });
    }
}

/// Records tracing fields as strings because these tests care about semantic content, not JSON formatting.
#[derive(Debug, Default)]
struct EventFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl tracing::field::Visit for EventFieldVisitor {
    /// Preserves string fields exactly as handler logs emitted them.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Preserves signed integers in decimal form for stable assertions.
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Preserves unsigned integers in decimal form for stable assertions.
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Falls back to debug formatting for field types without a more specific visitor hook.
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.fields.insert(
            field.name().to_string(),
            format!("{value:?}").trim_matches('"').to_string(),
        );
    }
}

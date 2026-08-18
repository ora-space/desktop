use crate::session::mapper::map_session;
use crate::session::ports::SessionRepository;
use crate::{ApplicationError, Clock};
use ora_contracts::{
    DeleteSessionRequest, DeleteSessionResponse, GetSessionRequest, GetSessionResponse,
    ListSessionsRequest, ListSessionsResponse, RenameSessionRequest, RenameSessionResponse,
};
use ora_domain::{SessionId, SessionTitle};

/// Handles one session lookup without depending on transport-specific concerns.
pub struct GetSessionHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetSessionHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetSessionHandler<Repository>
where
    Repository: SessionRepository,
{
    /// Loads one visible session or returns a stable not-found application error.
    pub fn handle(
        &self,
        request: GetSessionRequest,
    ) -> Result<GetSessionResponse, ApplicationError> {
        let session_id = SessionId::new(request.session_id);
        match self
            .repository
            .find_session(&session_id)
            .map_err(ApplicationError::from_session_repository_error)?
        {
            Some(session) => Ok(GetSessionResponse {
                session: map_session(session),
            }),
            None => Err(ApplicationError::SessionNotFound {
                session_id: session_id.to_string(),
            }),
        }
    }
}

/// Handles session listing without depending on transport-specific concerns.
pub struct ListSessionsHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListSessionsHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListSessionsHandler<Repository>
where
    Repository: SessionRepository,
{
    /// Lists every visible session and maps each one into the shared contract view.
    pub fn handle(
        &self,
        _request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, ApplicationError> {
        let sessions = self
            .repository
            .list_sessions()
            .map_err(ApplicationError::from_session_repository_error)?;
        Ok(ListSessionsResponse {
            sessions: sessions.into_iter().map(map_session).collect(),
        })
    }
}

/// Handles Ora-only session deletion without deleting provider-owned history.
pub struct DeleteSessionHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteSessionHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteSessionHandler<Repository, ClockSource>
where
    Repository: SessionRepository,
    ClockSource: Clock,
{
    /// Soft-deletes one stopped Ora session record.
    pub fn handle(
        &self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, ApplicationError> {
        let session_id = SessionId::new(request.session_id);
        let deleted = self
            .repository
            .soft_delete_session(&session_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_session_repository_error)?;
        if deleted {
            Ok(DeleteSessionResponse {
                session_id: session_id.to_string(),
            })
        } else {
            Err(ApplicationError::SessionNotFound {
                session_id: session_id.to_string(),
            })
        }
    }
}

/// Handles user-driven session title updates without depending on transport concerns.
pub struct RenameSessionHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> RenameSessionHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> RenameSessionHandler<Repository, ClockSource>
where
    Repository: SessionRepository,
    ClockSource: Clock,
{
    /// Replaces one session's display title after domain validation.
    pub fn handle(
        &self,
        request: RenameSessionRequest,
    ) -> Result<RenameSessionResponse, ApplicationError> {
        let session_id = SessionId::new(request.session_id);
        let title = SessionTitle::parse(request.title)
            .map_err(ApplicationError::from_session_title_error)?;
        match self.repository.update_session_title(
            &session_id,
            &title,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => Ok(RenameSessionResponse {
                session: map_session(session),
            }),
            Err(error) => {
                // RETURNING no rows is how SQLite reports both never-created and
                // already-deleted sessions; look up afterwards so that miss maps
                // to not-found instead of a generic repository failure.
                if self
                    .repository
                    .find_session(&session_id)
                    .map_err(ApplicationError::from_session_repository_error)?
                    .is_none()
                {
                    return Err(ApplicationError::SessionNotFound {
                        session_id: session_id.to_string(),
                    });
                }
                Err(ApplicationError::from_session_repository_error(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepositoryError;
    use ora_contracts::{
        RenameSessionResponse, Session as ContractSession,
        SessionHistoryState as ContractHistoryState, SessionStatus as ContractSessionStatus,
    };
    use ora_domain::{
        AgentCli, AgentRef, AuditFields, HistoryState, MAX_SESSION_TITLE_CHARS, Session,
        SessionStatus,
    };
    use pretty_assertions::assert_eq;
    use std::sync::Mutex;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now_timestamp_millis(&self) -> i64 {
            100
        }
    }

    struct MemorySessionRepository {
        sessions: Mutex<Vec<Session>>,
    }

    impl SessionRepository for MemorySessionRepository {
        fn create_session(&self, session: Session) -> Result<Session, RepositoryError> {
            self.sessions.lock().unwrap().push(session.clone());
            Ok(session)
        }

        fn find_session(&self, session_id: &SessionId) -> Result<Option<Session>, RepositoryError> {
            Ok(self
                .sessions
                .lock()
                .unwrap()
                .iter()
                .find(|session| session.id == *session_id)
                .cloned())
        }

        fn list_sessions(&self) -> Result<Vec<Session>, RepositoryError> {
            Ok(self.sessions.lock().unwrap().clone())
        }

        fn update_session_title(
            &self,
            session_id: &SessionId,
            title: &SessionTitle,
            now: i64,
        ) -> Result<Session, RepositoryError> {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(session) = sessions
                .iter_mut()
                .find(|session| session.id == *session_id)
            else {
                return Err(RepositoryError::from_message("session not found"));
            };
            *session = session.clone().with_title(Some(title.clone()));
            session.audit_fields.updated_at = now;
            Ok(session.clone())
        }

        fn update_session_status(
            &self,
            _session_id: &SessionId,
            _status: SessionStatus,
            _now: i64,
        ) -> Result<Session, RepositoryError> {
            unreachable!()
        }

        fn update_session_binding(
            &self,
            _session_id: &SessionId,
            _agent_ref: AgentRef,
            _agent_session_id: &str,
            _now: i64,
        ) -> Result<Session, RepositoryError> {
            unreachable!()
        }

        fn update_session_history_state(
            &self,
            _session_id: &SessionId,
            _history_state: &HistoryState,
            _now: i64,
        ) -> Result<Session, RepositoryError> {
            unreachable!()
        }

        fn soft_delete_session(
            &self,
            _session_id: &SessionId,
            _deleted_at: i64,
        ) -> Result<bool, RepositoryError> {
            unreachable!()
        }
    }

    /// Verifies blank titles fail validation before the repository is asked to mutate.
    #[test]
    fn rename_rejects_blank_titles() {
        let error = rename_handler(sample_session())
            .handle(RenameSessionRequest {
                session_id: "s1".to_owned(),
                title: "   ".to_owned(),
            })
            .unwrap_err();
        assert_eq!(error, ApplicationError::SessionTitleBlank);
    }

    /// Verifies overlong titles fail validation before the repository is asked to mutate.
    #[test]
    fn rename_rejects_titles_over_the_character_limit() {
        let error = rename_handler(sample_session())
            .handle(RenameSessionRequest {
                session_id: "s1".to_owned(),
                title: "x".repeat(MAX_SESSION_TITLE_CHARS + 1),
            })
            .unwrap_err();
        assert_eq!(error, ApplicationError::SessionTitleTooLong);
    }

    /// Verifies a valid title is persisted and returned as the contract session snapshot.
    #[test]
    fn rename_persists_a_trimmed_title() {
        let response = rename_handler(sample_session())
            .handle(RenameSessionRequest {
                session_id: "s1".to_owned(),
                title: "  Review auth  ".to_owned(),
            })
            .expect("rename succeeds");
        assert_eq!(
            response,
            RenameSessionResponse {
                session: ContractSession {
                    id: "s1".to_owned(),
                    task_id: "t1".to_owned(),
                    title: Some("Review auth".to_owned()),
                    agent_ref: "ora-space.opencode".to_string(),
                    status: ContractSessionStatus::Running,
                    history_state: ContractHistoryState::Writable,
                },
            },
        );
    }

    /// Verifies a missing row is reported as not-found instead of a repository failure.
    #[test]
    fn rename_missing_session_returns_not_found() {
        let error = RenameSessionHandler::new(
            MemorySessionRepository {
                sessions: Mutex::new(vec![]),
            },
            FixedClock,
        )
        .handle(RenameSessionRequest {
            session_id: "missing".to_owned(),
            title: "Review auth".to_owned(),
        })
        .unwrap_err();
        assert_eq!(
            error,
            ApplicationError::SessionNotFound {
                session_id: "missing".to_owned(),
            },
        );
    }

    fn sample_session() -> Session {
        Session::new(
            SessionId::new("s1"),
            ora_domain::TaskId::new("t1"),
            AgentCli::OpenCode.agent_ref(),
            "provider-1",
            SessionStatus::Running,
            AuditFields::new(1, 1, false),
        )
    }

    fn rename_handler(
        session: Session,
    ) -> RenameSessionHandler<MemorySessionRepository, FixedClock> {
        RenameSessionHandler::new(
            MemorySessionRepository {
                sessions: Mutex::new(vec![session]),
            },
            FixedClock,
        )
    }
}

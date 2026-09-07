use super::Sessions;
use crate::{AppEventHub, Backend, test_backend::backend_paths};
use ora_application::SessionRepository;
use ora_contracts::*;
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, SqliteSessionRepository, default_migration_catalog,
};
use ora_domain::{AgentRef, AuditFields, Session, SessionId, WorkspaceId};
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

struct Fixture {
    sessions: Arc<Sessions>,
    events: Arc<AppEventHub>,
    database: PathBuf,
    _temporary: TempDir,
}

impl Fixture {
    /// Seeds an ordinary stopped session with no actor, retaining only the consumer interfaces.
    fn stopped() -> Self {
        let temporary = TempDir::new().expect("session fixture");
        let backend =
            Backend::open(backend_paths(temporary.path(), temporary.path())).expect("backend");
        backend
            .projects()
            .create(CreateProjectRequest {
                name: "Session fixture".to_string(),
                main_workspace_path: temporary.path().to_string_lossy().into_owned(),
            })
            .expect("project");
        let workspace_id = backend
            .workspaces()
            .list(ListWorkspacesRequest {})
            .expect("workspaces")
            .workspaces
            .remove(0)
            .id;
        let database = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("fixture connection to the composed database");
        SqliteSessionRepository::new(pool)
            .create_session(Session::new(
                SessionId::new("session-1"),
                WorkspaceId::new(workspace_id),
                AgentRef::parse("ora-space.fixture").expect("agent identity"),
                "provider-session",
                ora_domain::SessionStatus::Stopped,
                AuditFields::new(1, 1, /*is_deleted*/ false),
            ))
            .expect("session row");
        Self {
            sessions: backend.sessions(),
            events: backend.app_events(),
            database,
            _temporary: temporary,
        }
    }
}

/// Includes repository setup and every emitting operation in the same scoped TRACE subscriber.
fn run_test(test: impl std::future::Future<Output = ()>) {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(test);
    });
}

/// A missing actor does not roll back a persisted user title, and subscribers can read it as soon
/// as its notification arrives through the shared application event source.
#[test]
fn rename_without_an_actor_commits_before_notifying_subscribers() {
    run_test(async {
        let fixture = Fixture::stopped();
        let mut events = fixture.events.subscribe();
        assert_eq!(
            events.recv().await.transpose().expect("ready"),
            Some(AppEvent::Ready)
        );
        let response = fixture
            .sessions
            .rename(RenameSessionRequest {
                session_id: "session-1".to_string(),
                title: "User title".to_string(),
            })
            .await
            .expect("rename without live actor");
        assert_eq!(
            events.recv().await.transpose().expect("title notification"),
            Some(AppEvent::SessionTitleUpdated {
                session_id: "session-1".to_string()
            })
        );
        assert_eq!(
            fixture
                .sessions
                .get(GetSessionRequest {
                    session_id: "session-1".to_string()
                })
                .expect("renamed session"),
            GetSessionResponse {
                session: response.session.clone()
            }
        );
        assert_eq!(
            fixture
                .sessions
                .list(ListSessionsRequest {})
                .expect("ordinary session list"),
            ListSessionsResponse {
                sessions: vec![response.session]
            }
        );
    });
}

/// A real SQLite title write failure preserves the row and emits no misleading invalidation.
#[test]
fn failed_rename_preserves_the_session_and_does_not_publish_success() {
    run_test(async {
        let fixture = Fixture::stopped();
        let before = fixture
            .sessions
            .get(GetSessionRequest {
                session_id: "session-1".to_string(),
            })
            .expect("original session");
        let mut events = fixture.events.subscribe();
        assert_eq!(
            events.recv().await.transpose().expect("ready"),
            Some(AppEvent::Ready)
        );
        rusqlite::Connection::open(&fixture.database).expect("fault injection connection").execute_batch(
            "CREATE TRIGGER reject_session_title BEFORE UPDATE ON sessions BEGIN SELECT RAISE(FAIL, 'fixture write failure'); END;",
        ).expect("inject write failure");
        let error = fixture
            .sessions
            .rename(RenameSessionRequest {
                session_id: "session-1".to_string(),
                title: "Rejected title".to_string(),
            })
            .await
            .expect_err("rename must fail");
        assert_eq!(
            error.public_error(),
            &PublicError::InternalError(EmptyErrorParams {})
        );
        assert_eq!(
            fixture
                .sessions
                .get(GetSessionRequest {
                    session_id: "session-1".to_string()
                })
                .expect("unchanged session"),
            before
        );
        assert!(
            events.try_recv().is_none(),
            "failed write has no success notification"
        );
    });
}

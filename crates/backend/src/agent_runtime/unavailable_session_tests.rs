//! Covers loading Ora-owned history when the agent bound to a session is unavailable.

use super::{AgentRuntimeManager, AgentRuntimeSetup};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::user_config::UserConfigApi;
use agent_client_protocol_schema::v1::{ContentBlock, StopReason, TextContent};
use ora_application::{ProjectRepository, SessionRepository};
use ora_contracts::{LoadSessionEvent, LoadSessionRequest};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteProjectRepository,
    SqliteSessionRepository, SqliteWorkspaceRepository, default_migration_catalog,
};
use ora_domain::{
    AgentRef, AuditFields, Project, ProjectId, Session, SessionId, SessionStatus, WorkspaceLocation,
};
use ora_history::FixedHistoryClock;
use ora_logging::with_trace_logging;
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use time::macros::datetime;

const SESSION_ID: &str = "session-with-missing-agent";
const MISSING_AGENT: &str = "ora-space.opencode";

/// Opens a migrated repository used by the runtime and plugin host.
fn test_pool(root: &Path) -> RepositoryPool {
    DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("test.sqlite")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool")
}

/// Builds a runtime over an empty installed-plugin layout.
fn test_manager(root: &Path, pool: &RepositoryPool, scheduler: Scheduler) -> AgentRuntimeManager {
    let app_events = AppEventHub::new().publisher();
    let plugin_host = Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            PathBuf::from("deno"),
            SystemClock,
            app_events.clone(),
            Arc::new(UserConfigApi::new(pool.clone())),
        )
        .expect("open plugin host"),
    );
    AgentRuntimeManager::new(AgentRuntimeSetup {
        plugin_host,
        pool: pool.clone(),
        home_directory: root.to_path_buf(),
        relative_path_base: root.to_path_buf(),
        sessions_root: root.join("sessions"),
        clock: SystemClock,
        scheduler,
        app_events,
    })
    .expect("build agent runtime manager")
}

/// Persists one stopped session and the Ora-owned user turn it recorded previously.
fn seed_session(root: &Path, pool: &RepositoryPool) {
    let workspace_path = root.join("project");
    std::fs::create_dir_all(&workspace_path).expect("create project directory");
    SqliteProjectRepository::new(pool.clone())
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Missing agent",
                AuditFields::new(1, 1, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .expect("create project");
    let workspace = SqliteWorkspaceRepository::new(pool.clone())
        .find_main_workspace(&ProjectId::new("project-1"))
        .expect("query main workspace")
        .expect("main workspace");
    let session = Session::new(
        SessionId::new(SESSION_ID),
        workspace.id,
        AgentRef::parse(MISSING_AGENT).expect("agent identity"),
        "provider-session-1",
        SessionStatus::Stopped,
        AuditFields::new(2, 2, false),
    );
    SqliteSessionRepository::new(pool.clone())
        .create_session(session)
        .expect("create session");

    let mut recorder = super::history::SessionRecorder::open(
        &root.join("sessions"),
        SESSION_ID,
        0,
        &ora_domain::HistoryState::Writable,
        FixedHistoryClock::new(datetime!(2026-09-03 11:20:02.558 +08:00)),
    )
    .expect("open session history");
    assert_eq!(
        recorder.record_prompt(&[ContentBlock::Text(TextContent::new("previous question"))]),
        super::history::RecordOutcome::Continued,
    );
    assert_eq!(
        recorder.record_turn_end(StopReason::EndTurn),
        super::history::RecordOutcome::Continued,
    );
}

/// A removed agent must not hide history that Ora can replay without that agent.
#[test]
fn loads_recorded_history_without_the_session_agent() {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
            .block_on(async {
                let temporary = TempDir::new().expect("create test directory");
                let pool = test_pool(temporary.path());
                seed_session(temporary.path(), &pool);
                let scheduler = Scheduler::new(chrono_tz::Asia::Shanghai);
                let manager = test_manager(temporary.path(), &pool, scheduler.clone());

                let mut stream = manager
                    .load_session(LoadSessionRequest {
                        session_id: SESSION_ID.to_string(),
                    })
                    .await
                    .expect("Ora history does not require the removed agent");
                let mut events = Vec::new();
                while let Some(event) = stream.recv().await {
                    events.push(event.expect("recorded history event"));
                }

                assert_eq!(
                    events,
                    vec![
                        LoadSessionEvent::SessionUpdate {
                            update:
                                agent_client_protocol_schema::v1::SessionUpdate::UserMessageChunk(
                                    agent_client_protocol_schema::v1::ContentChunk::new(
                                        ContentBlock::Text(TextContent::new("previous question")),
                                    ),
                                ),
                        },
                        LoadSessionEvent::TurnEnded {
                            stop_reason: StopReason::EndTurn,
                        },
                        LoadSessionEvent::Completed,
                    ],
                );
                scheduler.shutdown().await;
            });
    });
}

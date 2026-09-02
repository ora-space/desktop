//! Covers opening a conversation without the agent that produced it.
//!
//! The transcript is Ora's own record, so reading one has to survive the agent behind it going
//! away entirely — an uninstalled plugin, a CLI that cannot start. These tests pin that a load
//! asks the runtime for nothing: no provider session, no actor, no lifecycle change. What the
//! agent is needed for is the next prompt, which acquires it then.

use super::history::SessionRecorder;
use super::{AgentRuntimeManager, AgentRuntimeSetup, RuntimeActorHandle, RuntimeCommand};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::user_config::UserConfigApi;
use agent_client_protocol_schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent,
};
use ora_application::{ProjectRepository, SessionRepository};
use ora_contracts::{LoadSessionEvent, LoadSessionRequest};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteProjectRepository,
    SqliteSessionRepository, SqliteWorkspaceRepository, default_migration_catalog,
};
use ora_domain::{
    AgentRef, AuditFields, HistoryState, Project, ProjectId, Session, SessionId, SessionStatus,
    WorkspaceLocation,
};
use ora_history::FixedHistoryClock;
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use time::macros::datetime;
use tokio::sync::mpsc;

const SESSION_ID: &str = "session-1";
/// An agent identity no package in these fixtures supplies, so nothing supervises it.
const AGENT: &str = "ora-space.opencode";

fn test_pool(root: &Path) -> RepositoryPool {
    DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("test.sqlite")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool")
}

fn test_manager(root: &Path, pool: &RepositoryPool, scheduler: Scheduler) -> AgentRuntimeManager {
    let plugin_host = Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            PathBuf::from("deno"),
            SystemClock,
            AppEventHub::new().publisher(),
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
        app_events: AppEventHub::new().publisher(),
    })
    .expect("build agent runtime manager")
}

/// Persists one stopped session, bound to an agent this installation cannot reach.
fn seed_session(root: &Path, pool: &RepositoryPool) -> Session {
    let checkout = root.join("project");
    std::fs::create_dir_all(&checkout).expect("create project checkout");
    SqliteProjectRepository::new(pool.clone())
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Fixture project",
                AuditFields::new(1, 1, false),
            ),
            WorkspaceLocation::local_filesystem(checkout.to_string_lossy()),
        )
        .expect("create project");
    let workspace = SqliteWorkspaceRepository::new(pool.clone())
        .find_main_workspace(&ProjectId::new("project-1"))
        .expect("read main workspace")
        .expect("project has a main workspace");
    SqliteSessionRepository::new(pool.clone())
        .create_session(Session::new(
            SessionId::new(SESSION_ID),
            workspace.id,
            AgentRef::parse(AGENT).expect("agent identity"),
            "provider-session-1",
            SessionStatus::Stopped,
            AuditFields::new(2, 2, false),
        ))
        .expect("create session")
}

/// Records the file one finished turn leaves behind, through the writer production uses.
///
/// The clock is fixed rather than local: these tests assert on what a load streams, and the
/// process-wide local clock the runtime installs is not initialized for a unit test.
fn record_conversation(sessions_root: &Path, session: &Session) {
    let mut recorder = SessionRecorder::open(
        sessions_root,
        SESSION_ID,
        0,
        &HistoryState::Writable,
        FixedHistoryClock::new(datetime!(2026-09-02 09:15:00.000 +08:00)),
    )
    .expect("open recorder");
    recorder.record_meta(session, Path::new("/project"));
    recorder.record_prompt(&[ContentBlock::Text(TextContent::new("hello"))]);
    recorder.record_update(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new("hi")),
    )));
    recorder.record_turn_end(StopReason::EndTurn);
}

/// Collects a finite load stream, failing the test on the first error it carries.
async fn drain(mut stream: super::SessionEventStream<LoadSessionEvent>) -> Vec<LoadSessionEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event.expect("load streams the recorded conversation"));
    }
    events
}

/// The conversation of a session whose agent is gone is served in full.
///
/// This is the whole point of owning the record: an uninstalled plugin takes the ability to
/// continue the conversation, never the ability to read it.
#[tokio::test]
async fn a_session_whose_agent_is_unreachable_still_serves_its_transcript() {
    let temporary = TempDir::new().expect("create test directory");
    let pool = test_pool(temporary.path());
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let manager = test_manager(temporary.path(), &pool, scheduler.clone());
    let session = seed_session(temporary.path(), &pool);
    record_conversation(&temporary.path().join("sessions"), &session);

    let stream = manager
        .load_session(LoadSessionRequest {
            session_id: SESSION_ID.to_string(),
        })
        .await
        .expect("an unreachable agent does not stop a load");

    assert_eq!(
        drain(stream).await,
        vec![
            LoadSessionEvent::SessionUpdate {
                update: SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new("hello")
                ))),
            },
            LoadSessionEvent::SessionUpdate {
                update: SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new("hi")
                ))),
            },
            LoadSessionEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
            },
            LoadSessionEvent::Completed,
        ],
    );
    scheduler.shutdown().await;
}

/// Reading a conversation registers nothing and moves no lifecycle state.
///
/// A reader may never send anything, so a load that installed an actor or marked the session
/// running would claim a provider nobody asked for — and, through the running guard, keep the
/// project it belongs to from being deleted while someone reads old messages.
#[tokio::test]
async fn reading_a_conversation_leaves_the_session_unattached() {
    let temporary = TempDir::new().expect("create test directory");
    let pool = test_pool(temporary.path());
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let manager = test_manager(temporary.path(), &pool, scheduler.clone());
    let session = seed_session(temporary.path(), &pool);
    record_conversation(&temporary.path().join("sessions"), &session);

    drain(
        manager
            .load_session(LoadSessionRequest {
                session_id: SESSION_ID.to_string(),
            })
            .await
            .expect("load the recorded conversation"),
    )
    .await;

    assert!(
        manager
            .inner
            .actors
            .read()
            .expect("actor registry")
            .is_empty(),
        "reading a conversation must not install an actor",
    );
    assert_eq!(
        SqliteSessionRepository::new(pool.clone())
            .find_session(&SessionId::new(SESSION_ID))
            .expect("read session"),
        Some(session),
    );
    scheduler.shutdown().await;
}

/// A session that was never prompted opens as an empty conversation rather than a failure.
#[tokio::test]
async fn a_session_with_no_recorded_history_completes_immediately() {
    let temporary = TempDir::new().expect("create test directory");
    let pool = test_pool(temporary.path());
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let manager = test_manager(temporary.path(), &pool, scheduler.clone());
    seed_session(temporary.path(), &pool);

    let stream = manager
        .load_session(LoadSessionRequest {
            session_id: SESSION_ID.to_string(),
        })
        .await
        .expect("load a session that has said nothing");

    assert_eq!(drain(stream).await, vec![LoadSessionEvent::Completed]);
    scheduler.shutdown().await;
}

/// A live actor answers its own loads instead of the detached reader.
///
/// Only the actor knows the durable cutoff and the records of a turn still streaming, so routing
/// past it would show a reader a conversation that stops short of what is on screen elsewhere.
#[tokio::test]
async fn a_live_actor_answers_its_own_load() {
    let temporary = TempDir::new().expect("create test directory");
    let pool = test_pool(temporary.path());
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let manager = test_manager(temporary.path(), &pool, scheduler.clone());
    let session = seed_session(temporary.path(), &pool);
    record_conversation(&temporary.path().join("sessions"), &session);
    let (commands, mut received) = mpsc::unbounded_channel();
    manager
        .inner
        .actors
        .write()
        .expect("actor registry")
        .insert(SessionId::new(SESSION_ID), RuntimeActorHandle { commands });

    let loading = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .load_session(LoadSessionRequest {
                    session_id: SESSION_ID.to_string(),
                })
                .await
                .map(drop)
        }
    });
    let command = received.recv().await.expect("the actor is asked to load");
    let RuntimeCommand::Load { accepted, .. } = command else {
        panic!("a load must reach the actor as a load command");
    };
    accepted
        .send(Ok(()))
        .map_err(drop)
        .expect("the load is still waiting for admission");

    loading
        .await
        .expect("the load task runs to completion")
        .expect("an admitted load returns its stream");
    scheduler.shutdown().await;
}

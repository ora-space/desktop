//! Covers opening a conversation without the agent that produced it.
//!
//! The transcript is Ora's own record, so reading one has to survive the agent behind it going
//! away entirely — an uninstalled plugin, a CLI that cannot start. These tests pin that a load
//! asks the runtime for nothing: no provider session, no actor, no lifecycle change. What the
//! agent is needed for is the next prompt, which acquires it then.

use super::history::SessionRecorder;
use super::{AgentRuntimeManager, RuntimeActorHandle, RuntimeCommand};
use crate::MemorySessionStore;
use crate::host::SessionStore;
use crate::test_host::{RecordedEvents, TestHost, test_runtime};
use agent_client_protocol_schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent,
};
use ora_contracts::{LoadSessionEvent, LoadSessionRequest};
use ora_domain::{
    AgentRef, AuditFields, HistoryState, Session, SessionId, SessionStatus, WorkspaceId,
};
use ora_history::FixedHistoryClock;
use ora_logging::with_trace_logging;
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use std::path::Path;
use tempfile::TempDir;
use time::format_description::well_known::Rfc3339;
use time::macros::datetime;
use tokio::sync::mpsc;

const SESSION_ID: &str = "session-1";
/// An agent identity no package in these fixtures supplies, so nothing supervises it.
const AGENT: &str = "ora-space.opencode";
const HISTORY_CLOCK: time::OffsetDateTime = datetime!(2026-09-02 09:15:00.000 +08:00);

/// RFC 3339 stamp the fixture recorder writes onto every history line.
fn history_recorded_at() -> String {
    HISTORY_CLOCK
        .format(&Rfc3339)
        .expect("format fixture timestamp")
}

fn test_manager(
    root: &Path,
    store: &MemorySessionStore,
    scheduler: Scheduler,
) -> AgentRuntimeManager<TestHost> {
    // These tests drive the runtime directly rather than through a host, so nothing else has
    // installed the process-wide local clock the runtime timestamps rows and logs with. It is
    // idempotent, and the whole binary shares one: without it these tests pass only when some
    // other test in the same process happens to be scheduled first.
    ora_logging::initialize_test_clock();
    test_runtime(root, Vec::new(), store.clone(), scheduler).manager
}

/// Persists one stopped session, bound to an agent this installation cannot reach.
fn seed_session(store: &MemorySessionStore) -> Session {
    store
        .create_session(Session::new(
            SessionId::new(SESSION_ID),
            WorkspaceId::new("workspace-1"),
            AgentRef::parse(AGENT).expect("agent identity"),
            "provider-session-1",
            SessionStatus::Stopped,
            ora_domain::SessionMcpSelection::Automatic,
            AuditFields::new(2, 2, false),
        ))
        .expect("create session")
}

/// Records the file one finished turn leaves behind, through the writer production uses.
///
/// The clock is fixed rather than local so the recorded timestamps are the same on every run and
/// on every machine; what a load streams is what these tests assert on.
fn record_conversation(sessions_root: &Path, session: &Session) {
    let mut recorder = SessionRecorder::open(
        sessions_root,
        SESSION_ID,
        0,
        &HistoryState::Writable,
        RecordedEvents::default(),
        FixedHistoryClock::new(HISTORY_CLOCK),
    )
    .expect("open recorder");
    recorder.record_meta(session, Path::new("/project"));
    recorder.record_prompt(&[ContentBlock::Text(TextContent::new("hello"))]);
    recorder.record_update(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new("hi")),
    )));
    recorder.record_turn_end_with(StopReason::EndTurn, None, None);
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
#[test]
fn a_session_whose_agent_is_unreachable_still_serves_its_transcript() {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
            .block_on(async {
                let temporary = TempDir::new().expect("create test directory");
                let store = MemorySessionStore::default();
                let scheduler = Scheduler::new(chrono_tz::UTC);
                let manager = test_manager(temporary.path(), &store, scheduler.clone());
                let session = seed_session(&store);
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
                            update: SessionUpdate::UserMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("hello"))
                            )),
                            recorded_at: Some(history_recorded_at()),
                            tool_timing: None,
                        },
                        LoadSessionEvent::SessionUpdate {
                            update: SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("hi"))
                            )),
                            recorded_at: Some(history_recorded_at()),
                            tool_timing: None,
                        },
                        LoadSessionEvent::TurnEnded {
                            stop_reason: StopReason::EndTurn,
                            recorded_at: Some(history_recorded_at()),
                        },
                        LoadSessionEvent::Completed,
                    ],
                );
                scheduler.shutdown().await;
            })
    });
}

/// Reading a conversation registers nothing and moves no lifecycle state.
///
/// A reader may never send anything, so a load that installed an actor or marked the session
/// running would claim a provider nobody asked for — and, through the running guard, keep the
/// project it belongs to from being deleted while someone reads old messages.
#[test]
fn reading_a_conversation_leaves_the_session_unattached() {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
            .block_on(async {
                let temporary = TempDir::new().expect("create test directory");
                let store = MemorySessionStore::default();
                let scheduler = Scheduler::new(chrono_tz::UTC);
                let manager = test_manager(temporary.path(), &store, scheduler.clone());
                let session = seed_session(&store);
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
                    store
                        .find_session(&SessionId::new(SESSION_ID))
                        .expect("read session"),
                    Some(session),
                );
                scheduler.shutdown().await;
            })
    });
}

/// A session that was never prompted opens as an empty conversation rather than a failure.
#[test]
fn a_session_with_no_recorded_history_completes_immediately() {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
            .block_on(async {
                let temporary = TempDir::new().expect("create test directory");
                let store = MemorySessionStore::default();
                let scheduler = Scheduler::new(chrono_tz::UTC);
                let manager = test_manager(temporary.path(), &store, scheduler.clone());
                seed_session(&store);

                let stream = manager
                    .load_session(LoadSessionRequest {
                        session_id: SESSION_ID.to_string(),
                    })
                    .await
                    .expect("load a session that has said nothing");

                assert_eq!(drain(stream).await, vec![LoadSessionEvent::Completed]);
                scheduler.shutdown().await;
            })
    });
}

/// A live actor answers its own loads instead of the detached reader.
///
/// Only the actor knows the durable cutoff and the records of a turn still streaming, so routing
/// past it would show a reader a conversation that stops short of what is on screen elsewhere.
#[test]
fn a_live_actor_answers_its_own_load() {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
            .block_on(async {
                let temporary = TempDir::new().expect("create test directory");
                let store = MemorySessionStore::default();
                let scheduler = Scheduler::new(chrono_tz::UTC);
                let manager = test_manager(temporary.path(), &store, scheduler.clone());
                let session = seed_session(&store);
                record_conversation(&temporary.path().join("sessions"), &session);
                let (commands, mut received) = mpsc::unbounded_channel();
                manager
                    .inner
                    .actors
                    .write()
                    .expect("actor registry")
                    .insert(
                        SessionId::new(SESSION_ID),
                        RuntimeActorHandle {
                            commands,
                            turn_cancel: Default::default(),
                        },
                    );

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
            })
    });
}

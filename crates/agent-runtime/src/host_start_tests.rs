//! Covers what a host that starts sessions on its own relies on: a session identity it chose can
//! never join an existing conversation, and waiting for an agent ends with an answer.

use crate::ErrorClassification;
use crate::MemorySessionStore;
use crate::host::SessionStore;
use crate::test_host::{TestRuntime, test_runtime};
use ora_contracts::{EmptyErrorParams, PublicError, StartSessionRequest};
use ora_domain::{
    AgentRef, AuditFields, Session, SessionId, SessionMcpSelection, SessionStatus, WorkspaceId,
};
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

/// Starting under an identity the store already holds is refused before any agent is reached,
/// and the stored conversation is left exactly as it was.
#[tokio::test]
async fn an_identity_already_in_use_is_refused() {
    let temporary = TempDir::new().expect("create test directory");
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let store = MemorySessionStore::default();
    let agent_ref = AgentRef::parse("official/ora-space.echo").expect("agent identity");
    let existing = store
        .create_session(Session::new(
            SessionId::new("execution-1"),
            WorkspaceId::new("workspace-1"),
            agent_ref.clone(),
            "provider-1",
            SessionStatus::Stopped,
            SessionMcpSelection::Automatic,
            AuditFields::new(1, 1, false),
        ))
        .expect("seed session");
    let TestRuntime { manager, .. } = test_runtime(
        temporary.path(),
        Vec::new(),
        store.clone(),
        scheduler.clone(),
    );

    let error = manager
        .start_session_with_id(
            SessionId::new("execution-1"),
            StartSessionRequest {
                workspace_id: "workspace-1".to_string(),
                agent_ref: agent_ref.to_string(),
                model: None,
            },
        )
        .await
        .expect_err("an identity in use is refused");

    assert_eq!(
        (
            error.classification(),
            store.list_sessions().expect("list sessions"),
        ),
        (ErrorClassification::Conflict, vec![existing]),
    );
    scheduler.shutdown().await;
}

/// Waiting for an agent nothing supplies answers at once instead of waiting forever, because no
/// supervisor exists to ever report it ready.
#[tokio::test]
async fn waiting_for_an_agent_that_is_not_installed_fails_at_once() {
    let temporary = TempDir::new().expect("create test directory");
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let TestRuntime { manager, .. } = test_runtime(
        temporary.path(),
        Vec::new(),
        MemorySessionStore::default(),
        scheduler.clone(),
    );

    let error = manager
        .wait_for_agent(&AgentRef::parse("official/ora-space.echo").expect("agent identity"))
        .await
        .expect_err("an agent nothing supplies is never ready");

    assert_eq!(
        error.public_error(),
        &PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
    );
    scheduler.shutdown().await;
}

/// Manager startup reconciles only visible running rows and preserves all other session facts.
#[tokio::test]
async fn manager_startup_reconciles_running_sessions_before_use() {
    let temporary = TempDir::new().expect("create test directory");
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let store = MemorySessionStore::default();
    let session = |id, status| {
        Session::new(
            SessionId::new(id),
            WorkspaceId::new("workspace-1"),
            AgentRef::parse("official/ora-space.echo").expect("agent identity"),
            "provider-1",
            status,
            SessionMcpSelection::Automatic,
            AuditFields::new(1, 1, false),
        )
    };
    let running = store
        .create_session(session("running", SessionStatus::Running))
        .expect("seed running row");
    let stopped = store
        .create_session(session("stopped", SessionStatus::Stopped))
        .expect("seed stopped row");
    let hidden = store
        .create_session(session("hidden", SessionStatus::Running))
        .expect("seed deleted row");
    store.soft_delete_session(&hidden.id, 2).expect("hide row");
    let before = crate::clock::SystemClock.now_timestamp_millis();
    let TestRuntime { manager, .. } = test_runtime(
        temporary.path(),
        Vec::new(),
        store.clone(),
        scheduler.clone(),
    );
    let after = crate::clock::SystemClock.now_timestamp_millis();
    let reconciled = store
        .find_session(&running.id)
        .expect("read running row")
        .expect("visible running row");
    let updated_at = reconciled.audit_fields.updated_at;
    assert!((before..=after).contains(&updated_at));
    assert_eq!(
        store.list_sessions().expect("list reconciled rows"),
        vec![
            running.with_status(SessionStatus::Stopped, updated_at),
            stopped
        ],
    );
    assert_eq!(
        store.find_session(&hidden.id).expect("read hidden row"),
        None
    );
    let once = store.list_sessions().expect("capture reconciled rows");
    drop(manager);
    let TestRuntime { manager, .. } = test_runtime(
        temporary.path(),
        Vec::new(),
        store.clone(),
        scheduler.clone(),
    );
    assert_eq!(
        store.list_sessions().expect("list rows after restart"),
        once
    );
    drop(manager);
    scheduler.shutdown().await;
}

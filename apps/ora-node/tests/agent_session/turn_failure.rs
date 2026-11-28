//! A failed turn's reason and its one settlement, driven through the echo agent.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md; each test names the case.

use super::support::*;
use ora_node::SessionCommand;
use ora_node_protocol::{AgentSessionEndReason, EndSessionReason, TurnFailureOutcome};
use pretty_assertions::assert_eq;

const FAILURE: Keep = Keep {
    on_cancel: false,
    on_failure: true,
};

/// Counts the turns the Thread shows as ended.
fn turn_ends(fixture: &Fixture) -> usize {
    fixture
        .ledger
        .journal()
        .iter()
        .filter(|entry| **entry == Entry::Thread("turnEnded".into()))
        .count()
}

/// Ends a kept session so the fixture leaves nothing running.
async fn end(fixture: &Fixture, sessions: &Sessions) -> AgentSessionEndReason {
    fixture.command(
        sessions,
        "end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await.reason
}

/// Case 11: a kept failure settles exactly once as `kept` before the next turn runs, and stays
/// the session's only conclusion for that turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_kept_failure_settles_kept_before_the_next_turn() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[request-failed]", FAILURE);
    let failures =
        until(|| (!fixture.ledger.failures().is_empty()).then(|| fixture.ledger.failures())).await;
    assert_eq!(
        failures,
        vec![("turn-1".to_string(), TurnFailureOutcome::Kept)]
    );
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
    );
    until(|| (turn_ends(&fixture) == 2).then_some(())).await;
    let reason = end(&fixture, &sessions).await;
    sessions.shutdown().await;
    assert_eq!(
        (reason, fixture.ledger.failures()),
        (
            AgentSessionEndReason::UserEnded,
            vec![("turn-1".to_string(), TurnFailureOutcome::Kept)],
        )
    );
}

/// Case 12: a failure that cannot keep (here, the plugin version has no verified turn boundary)
/// settles `session_ending` together with the end decision.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_that_cannot_keep_settles_session_ending() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[request-failed]", FAILURE);
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (
            ended.reason,
            ended.detail.as_deref(),
            fixture.ledger.failures(),
        ),
        (
            AgentSessionEndReason::AgentFailed,
            Some("turn_boundary_unverified"),
            vec![("turn-1".to_string(), TurnFailureOutcome::SessionEnding)],
        )
    );
}

/// The watchdog's windows, short enough for the give-up path to run within a test.
const WATCHDOG: &[std::time::Duration] = &[std::time::Duration::from_secs(1)];

/// The `turnEnded` failure the session's history recorded.
fn recorded_failure(fixture: &Fixture) -> Option<serde_json::Value> {
    fixture
        .history_lines()
        .iter()
        .find(|line| line.get("type").and_then(serde_json::Value::as_str) == Some("turnEnded"))
        .and_then(|line| line.get("failure").cloned())
}

/// Case 6: a watchdog cancel the Agent never answers ends the session with
/// `turn_cancel_unanswered`, for a session that did not choose failure keeping too, and records
/// `no_progress` with no answer; without failure keeping there is no settlement (case 14).
#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_watchdog_cancel_ends_as_turn_cancel_unanswered() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    sessions.set_test_prompt_inactivity_windows(WATCHDOG);
    fixture.start(&sessions, PLUGIN_VERSION, "[ignore-cancel]");
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (
            ended.reason,
            ended.detail.as_deref(),
            recorded_failure(&fixture),
            fixture.ledger.failures(),
        ),
        (
            AgentSessionEndReason::AgentFailed,
            Some("turn_cancel_unanswered"),
            Some(serde_json::json!({"code": "no_progress", "attempts": 1, "answer": "none"})),
            Vec::new(),
        )
    );
}

/// Case 6: a session that chose failure keeping ends the same way, after the turn's
/// `session_ending` settlement.
#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_watchdog_cancel_of_a_keeping_session_settles_session_ending() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    sessions.set_test_prompt_inactivity_windows(WATCHDOG);
    fixture.start_keeping(&sessions, "[ignore-cancel]", FAILURE);
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (
            ended.reason,
            ended.detail.as_deref(),
            fixture.ledger.failures(),
        ),
        (
            AgentSessionEndReason::AgentFailed,
            Some("turn_cancel_unanswered"),
            vec![("turn-1".to_string(), TurnFailureOutcome::SessionEnding)],
        )
    );
}

/// Case 2: ACP's `AuthRequired` code ends the session with `agent_authentication_required`
/// whatever the Agent's text, and the recorded failure says the same.
#[tokio::test(flavor = "multi_thread")]
async fn the_auth_required_code_ends_with_agent_authentication_required() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "[auth-code]");
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (
            ended.reason,
            ended.detail.as_deref(),
            recorded_failure(&fixture),
        ),
        (
            AgentSessionEndReason::AgentFailed,
            Some("agent_authentication_required"),
            Some(serde_json::json!({
                "code": "agent_authentication_required",
                "acp_code": -32000,
                "message": "Sign in to the provider first",
            })),
        )
    );
}

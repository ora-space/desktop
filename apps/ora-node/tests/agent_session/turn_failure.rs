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

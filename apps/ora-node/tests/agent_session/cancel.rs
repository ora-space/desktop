//! `CancelTurn` against the echo agent: where a cancel takes effect, its one settlement, and
//! whether the stopped turn leaves the session for the next one.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md; each test names
//! the case it covers.

use super::support::*;
use ora_node::{CommandSettlement, SessionCommand, SessionHost, TurnBoundaryPlatform};
use ora_node_protocol::{
    AgentSessionEndReason, EndSessionReason, ExecutionId, TurnCancelOutcome, TurnId,
};
use pretty_assertions::assert_eq;

#[path = "cancel/recovery.rs"]
mod recovery;

const CANCEL: Keep = Keep {
    on_cancel: true,
    on_failure: false,
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

/// Waits until the turn's prompt reached the Agent. The user message is recorded just before
/// the prompt is written, and an Agent chunk settles only once a later record follows it, so the
/// test waits for the message and then gives the write a moment.
async fn running(fixture: &Fixture, _turns_before: usize) {
    until(|| {
        fixture
            .ledger
            .events()
            .iter()
            .any(|event| {
                serde_json::to_string(&event.record)
                    .unwrap_or_default()
                    .contains("user_message_chunk")
            })
            .then_some(())
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

/// Waits for the cancel's settlement.
async fn settled(fixture: &Fixture, command: &str) -> TurnCancelOutcome {
    until(|| {
        fixture
            .ledger
            .cancels()
            .into_iter()
            .find_map(|(id, outcome)| (id == command).then_some(outcome))
    })
    .await
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

/// Case 1: a cancelled running turn ends `cancelled`, then its settlement, and the session takes
/// the next turn on the same provider session.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_running_turn_keeps_the_session_for_the_next_turn() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let outcome = settled(&fixture, "cancel-1").await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
    );
    until(|| (turn_ends(&fixture) == 2).then_some(())).await;
    let journal = fixture.ledger.journal();
    let cancelled_end = journal
        .iter()
        .position(|entry| *entry == Entry::Thread("turnEnded".into()))
        .expect("the cancelled turn ended");
    let settlement = journal
        .iter()
        .position(|entry| matches!(entry, Entry::Cancel(..)))
        .expect("the cancel settled");
    assert_eq!(
        (
            outcome,
            cancelled_end < settlement,
            fixture.ledger.ended(),
            end(&fixture, &sessions).await
        ),
        (
            TurnCancelOutcome::Stopped,
            true,
            None,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Case 2: a turn queued behind a running one is dropped as soon as its cancel takes effect,
/// without waiting for the running turn, and is never sent to the Agent.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_turn_is_dropped_at_once() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "never")),
    );
    fixture.command(
        &sessions,
        "cancel-2",
        SessionCommand::CancelTurn(TurnId::new("turn-2")),
    );
    let outcome = settled(&fixture, "cancel-2").await;
    let still_running = fixture.ledger.ended().is_none() && turn_ends(&fixture) == 0;
    let reason = end(&fixture, &sessions).await;
    let settlements = fixture.ledger.settlements();
    assert_eq!(
        (outcome, still_running, reason, settlements[0].clone()),
        (
            TurnCancelOutcome::NotStarted,
            true,
            AgentSessionEndReason::UserEnded,
            ("command-2".to_string(), vec![CommandSettlement::Discarded])
        )
    );
    assert!(
        !fixture
            .history_lines()
            .iter()
            .any(|line| serde_json::to_string(line).unwrap().contains("never")),
        "the dropped turn never reached the Agent"
    );
}

/// Case 3: cancels of a finished turn and of a turn the session never accepted do nothing.
#[tokio::test(flavor = "multi_thread")]
async fn cancels_of_ended_or_unknown_turns_do_nothing() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "hello", CANCEL);
    until(|| (turn_ends(&fixture) == 1).then_some(())).await;
    fixture.command(
        &sessions,
        "cancel-ended",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    fixture.command(
        &sessions,
        "cancel-unknown",
        SessionCommand::CancelTurn(TurnId::new("turn-x")),
    );
    let ended = settled(&fixture, "cancel-ended").await;
    let unknown = settled(&fixture, "cancel-unknown").await;
    assert_eq!(
        (
            ended,
            unknown,
            fixture.ledger.ended(),
            end(&fixture, &sessions).await
        ),
        (
            TurnCancelOutcome::AlreadyEnded,
            TurnCancelOutcome::Unknown,
            None,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Case 10: an Agent version without a verified turn boundary ends the session after a cancel,
/// and the cancel learns that the session is ending.
#[tokio::test(flavor = "multi_thread")]
async fn an_unverified_agent_ends_the_session_after_a_cancel() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let ended = fixture.ended().await;
    assert_eq!(
        (ended.reason, ended.detail, fixture.ledger.cancels()),
        (
            AgentSessionEndReason::AgentFailed,
            Some("turn_boundary_unverified".to_string()),
            vec![("cancel-1".to_string(), TurnCancelOutcome::SessionEnding)]
        )
    );
}

/// Case 11: no answer by the deadline ends the session with `turn_cancel_unanswered`.
#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_cancel_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.grace = std::time::Duration::from_millis(500);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[ignore-cancel]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let ended = fixture.ended().await;
    assert_eq!(
        (ended.detail, fixture.ledger.cancels()),
        (
            Some("turn_cancel_unanswered".to_string()),
            vec![("cancel-1".to_string(), TurnCancelOutcome::SessionEnding)]
        )
    );
}

/// Cases 7, 9 and 22: a tool still running at the answer, or turn traffic within the quiet
/// window, ends the session; a session-level title in that window does not.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_quiet_settled_turn_keeps_the_session() {
    let mut details = Vec::new();
    for initial in ["[hold-running-tool]", "[hold-chatty]", "[hold-titled]"] {
        let mut fixture = Fixture::new();
        fixture.verify_turn_boundary();
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_keeping(&sessions, initial, CANCEL);
        running(&fixture, 0).await;
        fixture.command(
            &sessions,
            "cancel-1",
            SessionCommand::CancelTurn(TurnId::new("turn-1")),
        );
        let outcome = settled(&fixture, "cancel-1").await;
        let detail = if outcome == TurnCancelOutcome::SessionEnding {
            fixture.ended().await.detail
        } else {
            assert_eq!(fixture.ledger.ended(), None);
            end(&fixture, &sessions).await;
            None
        };
        details.push((initial, outcome, detail));
    }
    assert_eq!(
        details,
        vec![
            (
                "[hold-running-tool]",
                TurnCancelOutcome::SessionEnding,
                Some("turn_not_quiescent".to_string())
            ),
            (
                "[hold-chatty]",
                TurnCancelOutcome::SessionEnding,
                Some("turn_not_quiescent".to_string())
            ),
            ("[hold-titled]", TurnCancelOutcome::Stopped, None),
        ]
    );
}

/// Cases 8 and 26: an update naming a stopped turn's tool call in a later turn joins no turn and
/// ends the session with `turn_leak_detected`.
#[tokio::test(flavor = "multi_thread")]
async fn a_leaked_update_in_a_later_turn_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold-done-tool]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    assert_eq!(
        settled(&fixture, "cancel-1").await,
        TurnCancelOutcome::Stopped
    );
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
    );
    until(|| (turn_ends(&fixture) == 2).then_some(())).await;
    let events_before = fixture.ledger.events().len();
    fixture.command(
        &sessions,
        "command-3",
        SessionCommand::SubmitUserTurn(turn("turn-3", "[leak]")),
    );
    let ended = fixture.ended().await;
    let leaked_into_thread = fixture.ledger.events()[events_before..]
        .iter()
        .any(|event| {
            serde_json::to_string(&event.record)
                .unwrap()
                .contains("echo-tool")
        });
    assert_eq!(
        (ended.detail, leaked_into_thread),
        (Some("turn_leak_detected".to_string()), false)
    );
}

/// Case 21: an `EndSession` queued with a cancel wins, and the cancel is settled as
/// `session_ending` before the terminal.
#[tokio::test(flavor = "multi_thread")]
async fn end_session_wins_over_a_queued_cancel() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.ledger.accept(
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    fixture.ledger.accept(
        "end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    sessions.command_arrived(&ExecutionId::new(EXECUTION));
    let ended = fixture.ended().await;
    assert_eq!(
        (ended.reason, fixture.ledger.cancels()),
        (
            AgentSessionEndReason::UserEnded,
            vec![("cancel-1".to_string(), TurnCancelOutcome::SessionEnding)]
        )
    );
}

/// Cases 18 and 30: a failed turn keeps a session that chose to stay after failures, and still
/// ends one that chose only to stay after cancels.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_turn_keeps_only_a_session_that_chose_it() {
    let mut results = Vec::new();
    for keep in [
        CANCEL,
        Keep {
            on_cancel: false,
            on_failure: true,
        },
    ] {
        let mut fixture = Fixture::new();
        fixture.verify_turn_boundary();
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_keeping(&sessions, "[request-failed]", keep);
        until(|| (turn_ends(&fixture) == 1).then_some(())).await;
        let detail = if keep.on_failure {
            fixture.command(
                &sessions,
                "command-2",
                SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
            );
            until(|| (turn_ends(&fixture) == 2 || fixture.ledger.ended().is_some()).then_some(()))
                .await;
            let detail = fixture.ledger.ended().and_then(|ended| ended.detail);
            end(&fixture, &sessions).await;
            detail
        } else {
            fixture.ended().await.detail
        };
        results.push((keep.on_failure, detail));
    }
    assert_eq!(
        results,
        vec![(false, Some("agent_turn_failed".to_string())), (true, None)]
    );
}

/// An Agent that answers a written cancel with `end_turn` still answered the cancel: the turn is
/// reported `cancelled` and judged by D2, so with the empty turn-boundary list the session ends,
/// and only a verified version keeps it. Only an answer already readable before the cancel is a
/// race the Agent won; the runtime covers that case.
#[tokio::test(flavor = "multi_thread")]
async fn an_end_turn_answer_to_a_cancel_counts_as_cancelled() {
    let mut results = Vec::new();
    for verified in [false, true] {
        let mut fixture = Fixture::new();
        if verified {
            fixture.verify_turn_boundary();
        }
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_keeping(&sessions, "[hold-finishing]", CANCEL);
        running(&fixture, 0).await;
        fixture.command(
            &sessions,
            "cancel-1",
            SessionCommand::CancelTurn(TurnId::new("turn-1")),
        );
        let outcome = settled(&fixture, "cancel-1").await;
        let stop_reason = fixture
            .ledger
            .events()
            .iter()
            .find(|event| event.record.get("type").and_then(|t| t.as_str()) == Some("turnEnded"))
            .and_then(|event| event.record.get("stop_reason").cloned());
        let detail = if outcome == TurnCancelOutcome::SessionEnding {
            fixture.ended().await.detail
        } else {
            end(&fixture, &sessions).await;
            None
        };
        results.push((verified, outcome, stop_reason, detail));
    }
    assert_eq!(
        results,
        vec![
            (
                false,
                TurnCancelOutcome::SessionEnding,
                Some(serde_json::json!("cancelled")),
                Some("turn_boundary_unverified".to_string())
            ),
            (
                true,
                TurnCancelOutcome::Stopped,
                Some(serde_json::json!("cancelled")),
                None
            ),
        ]
    );
}

/// Case 5: two cancels of one turn get one conclusion, whether the second arrives while the
/// stop waits for the answer or within the quiet window, and both learn of a fallback end.
#[tokio::test(flavor = "multi_thread")]
async fn duplicate_cancels_share_one_conclusion() {
    let mut results = Vec::new();
    for (verified, second_in_quiet) in [(true, false), (true, true), (false, false)] {
        let mut fixture = Fixture::new();
        if verified {
            fixture.verify_turn_boundary();
            fixture.turn_cancel.quiet = std::time::Duration::from_secs(2);
        }
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_keeping(&sessions, "[hold]", CANCEL);
        running(&fixture, 0).await;
        fixture.ledger.accept(
            "cancel-1",
            SessionCommand::CancelTurn(TurnId::new("turn-1")),
        );
        if second_in_quiet {
            sessions.command_arrived(&ExecutionId::new(EXECUTION));
            // The answer is recorded; the stop now waits out the quiet window.
            until(|| (turn_ends(&fixture) == 1).then_some(())).await;
            assert!(fixture.ledger.cancels().is_empty());
        }
        fixture.ledger.accept(
            "cancel-2",
            SessionCommand::CancelTurn(TurnId::new("turn-1")),
        );
        sessions.command_arrived(&ExecutionId::new(EXECUTION));
        let first = settled(&fixture, "cancel-1").await;
        let second = settled(&fixture, "cancel-2").await;
        if fixture.ledger.ended().is_none() {
            end(&fixture, &sessions).await;
        }
        results.push((first, second));
    }
    assert_eq!(
        results,
        vec![
            (TurnCancelOutcome::Stopped, TurnCancelOutcome::Stopped),
            (TurnCancelOutcome::Stopped, TurnCancelOutcome::Stopped),
            (
                TurnCancelOutcome::SessionEnding,
                TurnCancelOutcome::SessionEnding
            ),
        ]
    );
}

/// Case 6: a cancel accepted while a failed turn waits out its quiet window finds the turn
/// already ended by its failure, and the session that chose to stay after failures stays.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_during_a_failed_turns_quiet_window_is_already_ended() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.quiet = std::time::Duration::from_secs(2);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(
        &sessions,
        "[request-failed]",
        Keep {
            on_cancel: true,
            on_failure: true,
        },
    );
    until(|| (turn_ends(&fixture) == 1).then_some(())).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let outcome = settled(&fixture, "cancel-1").await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
    );
    until(|| (turn_ends(&fixture) == 2 || fixture.ledger.ended().is_some()).then_some(())).await;
    assert_eq!(
        (
            outcome,
            fixture.ledger.ended(),
            end(&fixture, &sessions).await
        ),
        (
            TurnCancelOutcome::AlreadyEnded,
            None,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Case 8 (live preview): a later turn's pending update that names a stopped turn's tool call
/// never reaches the live preview, and the session ends as a leak without waiting for the
/// update to settle.
#[tokio::test(flavor = "multi_thread")]
async fn a_pending_leaked_update_never_reaches_the_preview() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold-done-tool]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    assert_eq!(
        settled(&fixture, "cancel-1").await,
        TurnCancelOutcome::Stopped
    );
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "[leak-running]")),
    );
    let mut previewed = false;
    let ended = until(|| {
        previewed |= sessions.preview_snapshots().iter().any(|preview| {
            serde_json::to_string(&preview.payload)
                .unwrap_or_default()
                .contains("echo-tool")
        });
        fixture.ledger.ended()
    })
    .await;
    assert_eq!(
        (ended.detail, previewed),
        (Some("turn_leak_detected".to_string()), false)
    );
}

/// Case 11 (deadline origin): the cancel deadline counts from the durable acceptance, so time the
/// conversation spent before reaching the cancel is part of G, not added to it.
#[tokio::test(flavor = "multi_thread")]
async fn the_cancel_deadline_counts_from_acceptance() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.grace = std::time::Duration::from_secs(2);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[ignore-cancel]", CANCEL);
    running(&fixture, 0).await;
    fixture.ledger.accept(
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let woken = std::time::Instant::now();
    // Accepted 1.5 s before the session could act on it: only 0.5 s of G is left.
    sessions.turn_cancel_accepted(
        &ExecutionId::new(EXECUTION),
        &ora_node_protocol::CommandId::new("cancel-1"),
        tokio::time::Instant::now() - std::time::Duration::from_millis(1500),
    );
    let outcome = settled(&fixture, "cancel-1").await;
    let elapsed = woken.elapsed();
    assert_eq!(outcome, TurnCancelOutcome::SessionEnding);
    assert!(
        elapsed < std::time::Duration::from_millis(1700),
        "settled {elapsed:?} after the wake, beyond what remained of G"
    );
}

/// Case 24 at the Node: a cancel accepted while the turn's admission commits stops the prompt
/// before it is written, so even with the empty turn-boundary list the turn ends `cancelled`,
/// then `stopped`, and the session takes the next turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_during_admission_keeps_the_session_with_the_empty_list() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "hello", CANCEL);
    until(|| (turn_ends(&fixture) == 1).then_some(())).await;
    let ledger = fixture.ledger.clone();
    fixture.ledger.on_executed("command-2", move || {
        ledger.accept(
            "cancel-2",
            SessionCommand::CancelTurn(TurnId::new("turn-2")),
        );
    });
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "never answered")),
    );
    let outcome = settled(&fixture, "cancel-2").await;
    let journal = fixture.ledger.journal();
    let turn_end = journal
        .iter()
        .rposition(|entry| *entry == Entry::Thread("turnEnded".into()))
        .expect("the cancelled turn ended");
    let settlement = journal
        .iter()
        .position(|entry| matches!(entry, Entry::Cancel(..)))
        .expect("the cancel settled");
    fixture.command(
        &sessions,
        "command-3",
        SessionCommand::SubmitUserTurn(turn("turn-3", "hello again")),
    );
    until(|| (turn_ends(&fixture) == 3).then_some(())).await;
    let answered = fixture.history_lines().iter().any(|line| {
        serde_json::to_string(line)
            .unwrap()
            .contains("echo: never answered")
    });
    assert_eq!(
        (
            outcome,
            turn_end < settlement,
            answered,
            fixture.ledger.ended(),
            end(&fixture, &sessions).await
        ),
        (
            TurnCancelOutcome::Stopped,
            true,
            false,
            None,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// The turn-boundary list matches the exact plugin version: another version of the same plugin
/// is not verified, so a cancel ends the session.
#[tokio::test(flavor = "multi_thread")]
async fn another_version_of_a_listed_plugin_is_not_verified() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.turn_boundary = vec![boundary("9.9.9", TurnBoundaryPlatform::Linux)];
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    assert_eq!(
        fixture.ended().await.detail,
        Some("turn_boundary_unverified".to_string())
    );
}

/// An end decision that cannot be written ends the session as the ledger's failure, and the
/// cancel is settled by the decision written once the ledger works again.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_end_decision_ends_with_ledger_unavailable() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.ledger.fail_next_decision();
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let ended = fixture.ended().await;
    assert_eq!(
        (ended.detail, fixture.ledger.cancels()),
        (
            Some("ledger_unavailable".to_string()),
            vec![("cancel-1".to_string(), TurnCancelOutcome::SessionEnding)]
        )
    );
}

/// Case 5 (quiet window edge): a cancel accepted within the quiet window whose wake the stop
/// never saw still merges into the stop when it settles, so both cancels share `stopped`.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_accepted_as_the_quiet_window_ends_shares_the_conclusion() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.quiet = std::time::Duration::from_secs(1);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    running(&fixture, 0).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    until(|| (turn_ends(&fixture) == 1).then_some(())).await;
    // Accepted, but its wake is lost: only the stop's own settlement boundary can find it.
    fixture.ledger.accept(
        "cancel-2",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let first = settled(&fixture, "cancel-1").await;
    let second = settled(&fixture, "cancel-2").await;
    end(&fixture, &sessions).await;
    assert_eq!(
        (first, second),
        (TurnCancelOutcome::Stopped, TurnCancelOutcome::Stopped)
    );
}

/// An acceptance time lives exactly as long as its cancel is pending: it goes once the cancel is
/// settled, whatever the outcome, and a duplicate of a settled cancel never adds one again.
#[tokio::test(flavor = "multi_thread")]
async fn acceptance_times_live_only_while_their_cancel_is_pending() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "hello", CANCEL);
    until(|| (turn_ends(&fixture) == 1).then_some(())).await;
    let execution = ExecutionId::new(EXECUTION);
    let command = ora_node_protocol::CommandId::new("cancel-x");
    fixture.ledger.accept(
        "cancel-x",
        SessionCommand::CancelTurn(TurnId::new("turn-x")),
    );
    sessions.turn_cancel_accepted(&execution, &command, tokio::time::Instant::now());
    assert_eq!(
        settled(&fixture, "cancel-x").await,
        TurnCancelOutcome::Unknown
    );
    let after_settlement = sessions.pending_cancel_acceptances();
    // The Controller resends the settled command; the ledger answers it as a duplicate.
    sessions.turn_cancel_accepted(&execution, &command, tokio::time::Instant::now());
    let after_duplicate = sessions.pending_cancel_acceptances();
    end(&fixture, &sessions).await;
    assert_eq!((after_settlement, after_duplicate), (0, 0));
}

//! Where the fence sits in the judgement of a stopped turn: before the quiet window, with a
//! scan-only re-check right before `stopped`, and how cancels and `EndSession` accepted while it
//! runs are served once it returns.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md; each test names the case it
//! covers.
use super::*;
use pretty_assertions::assert_eq;

/// Settles every cancel of the session and returns them in acceptance order once `count` did.
async fn cancels(fixture: &Fixture, count: usize) -> Vec<(String, TurnCancelOutcome)> {
    until(|| {
        let cancels = fixture.ledger.cancels();
        (cancels.len() == count).then_some(cancels)
    })
    .await
}

/// Cases 1 and 5: a helper in the Agent's own session that silently starts a `setsid` job while
/// the quiet window runs, after the fence came back clean, is caught by the re-check before the
/// stop settles: the session ends with `turn_processes_unstopped` instead of being kept, and its
/// stop kills the job.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_started_after_a_clean_fence_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.quiet = Duration::from_secs(1);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[late-job 400] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let job = pinned(&fixture.checkout(), "late-job.pid").await;
    let ended = fixture.ended().await;
    assert_eq!(
        (
            fixture.ledger.cancels(),
            ended.detail.as_deref(),
            job.has_exited().unwrap()
        ),
        (
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            Some("turn_processes_unstopped"),
            true
        )
    );
}

/// Case 5: the fence runs before the quiet window, so an Agent that reacts to its background
/// job dying with turn traffic, as Claude Code starts a turn by itself, ends the session with
/// `turn_not_quiescent`.
#[tokio::test(flavor = "multi_thread")]
async fn turn_traffic_caused_by_the_fence_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[watch-job] [hold]", CANCEL);
    let job = pinned(&fixture.checkout(), "watch-job.pid").await;
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let ended = fixture.ended().await;
    // The Agent's message is late turn traffic: it joins no turn, so only the end shows it.
    assert_eq!(
        (ended.detail.as_deref(), job.has_exited().unwrap()),
        (Some("turn_not_quiescent"), true)
    );
}

/// Case 5: a second cancel accepted while a fence that takes long but ends clean still runs
/// merges into the stop once it returns, and both settle `stopped`; the session is kept.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_accepted_during_a_clean_fence_settles_stopped() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_fence = Duration::from_secs(3);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[respawn-for 600] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    // The supervisor keeps restarting its job for 600 ms after the answer.
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel(&fixture, &sessions, "cancel-2");
    let settled = cancels(&fixture, 2).await;
    let kept = fixture.ledger.ended().is_none();
    assert_eq!(
        (settled, kept, end(&fixture, &sessions).await),
        (
            vec![
                ("cancel-1".to_owned(), TurnCancelOutcome::Stopped),
                ("cancel-2".to_owned(), TurnCancelOutcome::Stopped)
            ],
            true,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Cases 4 and 5: a second cancel accepted while a fence that never gets clean runs shares the
/// stop's conclusion: both settle `session_ending` and the session ends with
/// `turn_processes_unstopped`.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_accepted_during_an_unclean_fence_shares_its_end() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_fence = Duration::from_secs(1);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[respawn] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel(&fixture, &sessions, "cancel-2");
    let ended = fixture.ended().await;
    assert_eq!(
        (fixture.ledger.cancels(), ended.detail.as_deref()),
        (
            vec![
                ("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding),
                ("cancel-2".to_owned(), TurnCancelOutcome::SessionEnding)
            ],
            Some("turn_processes_unstopped")
        )
    );
}

/// Case 5: an `EndSession` accepted while the fence runs takes precedence once it returns,
/// whether the fence came back clean or not: the session ends as the user asked and the cancel
/// settles `session_ending`.
#[tokio::test(flavor = "multi_thread")]
async fn an_end_session_accepted_during_the_fence_takes_precedence() {
    for (marker, fence) in [
        ("[respawn-for 600] [hold]", Duration::from_secs(3)),
        ("[respawn] [hold]", Duration::from_secs(1)),
    ] {
        let mut fixture = Fixture::new();
        fixture.verify_turn_boundary();
        fixture.turn_cancel.process_fence = fence;
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_keeping(&sessions, marker, CANCEL);
        prompted(&fixture, "[hold]").await;
        cancel(&fixture, &sessions, "cancel-1");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let reason = end(&fixture, &sessions).await;
        assert_eq!(
            (marker, fixture.ledger.cancels(), reason),
            (
                marker,
                vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
                AgentSessionEndReason::UserEnded
            )
        );
    }
}

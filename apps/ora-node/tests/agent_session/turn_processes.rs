//! The turn process fence against the echo agent's real processes: what a stopped turn started in
//! sessions of its own dies before the session is kept, while the Agent's own processes, what the
//! Agent starts in their sessions, and what earlier turns left keep running.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md; each test names the case it
//! covers.

use super::support::*;
use ora_node::{SessionCommand, TurnBoundaryPlatform};
use ora_node_protocol::{AgentSessionEndReason, EndSessionReason, TurnCancelOutcome, TurnId};
use ora_utils::process::{LinuxPidFd, linux_process};
use pretty_assertions::assert_eq;
use std::path::Path;
use std::time::{Duration, Instant};

#[path = "turn_processes/limits.rs"]
mod limits;
#[path = "turn_processes/order.rs"]
mod order;
#[path = "turn_processes/scope.rs"]
mod scope;

const CANCEL: Keep = Keep {
    on_cancel: true,
    on_failure: false,
};

/// Pins the process whose PID the fixture recorded in `name`, once it appears.
async fn pinned(directory: &Path, name: &str) -> LinuxPidFd {
    let record = directory.join(name);
    let pid: u32 = until(|| std::fs::read_to_string(&record).ok()?.trim().parse().ok()).await;
    LinuxPidFd::from_observation(&linux_process(pid).unwrap()).unwrap()
}

/// Waits for the cancel's settlement and reports, at that moment, whether `job` still ran.
async fn settled_with(
    fixture: &Fixture,
    command: &str,
    job: &LinuxPidFd,
) -> (TurnCancelOutcome, bool) {
    until(|| {
        fixture
            .ledger
            .cancels()
            .into_iter()
            .find_map(|(id, outcome)| (id == command).then_some(outcome))
            .map(|outcome| (outcome, !job.has_exited().unwrap()))
    })
    .await
}

/// Waits until the prompt of the latest turn reached the Agent: its message was recorded, and
/// the write follows a moment later.
async fn prompted(fixture: &Fixture, text: &str) {
    until(|| {
        fixture
            .ledger
            .events()
            .iter()
            .any(|event| {
                serde_json::to_string(&event.record)
                    .unwrap_or_default()
                    .contains(text)
            })
            .then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// Waits until the Thread shows `count` ended turns.
async fn turns_ended(fixture: &Fixture, count: usize) {
    until(|| {
        (fixture
            .ledger
            .journal()
            .iter()
            .filter(|entry| **entry == Entry::Thread("turnEnded".into()))
            .count()
            == count)
            .then_some(())
    })
    .await;
}

/// Whether any recorded Agent message contains `text`.
fn answered(fixture: &Fixture, text: &str) -> bool {
    fixture.ledger.events().iter().any(|event| {
        serde_json::to_string(&event.record)
            .unwrap_or_default()
            .contains(text)
    })
}

/// Cancels the first turn.
fn cancel(fixture: &Fixture, sessions: &Sessions, command: &str) {
    fixture.command(
        sessions,
        command,
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
}

/// Ends a kept session.
async fn end(fixture: &Fixture, sessions: &Sessions) -> AgentSessionEndReason {
    fixture.command(
        sessions,
        "end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await.reason
}

/// Waits until a job that wrote `background-leak` after `delay` would have written it.
fn assert_never_wrote(checkout: &Path, since: Instant, delay: Duration) {
    std::thread::sleep(
        (since + delay + Duration::from_millis(500)).saturating_duration_since(Instant::now()),
    );
    assert!(!checkout.join("background-leak").exists(), "the job wrote");
}

/// Case 1: a kept cancel kills the turn's `setsid` tool job before the cancel settles `stopped`, so its
/// delayed write never happens, and the session takes the next turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_kept_cancel_kills_the_turns_setsid_job_before_it_settles() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    let began = Instant::now();
    fixture.start_keeping(&sessions, "[background 3000] [hold]", CANCEL);
    let job = pinned(&fixture.checkout(), "background.pid").await;
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let at_settlement = settled_with(&fixture, "cancel-1", &job).await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
    );
    turns_ended(&fixture, 2).await;
    let kept = fixture.ledger.ended().is_none();
    assert_never_wrote(&fixture.checkout(), began, Duration::from_millis(3000));
    assert_eq!(
        (at_settlement, kept, end(&fixture, &sessions).await),
        (
            (TurnCancelOutcome::Stopped, false),
            true,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Case 2: a helper the Agent starts in its own session during the turn, as a lazily started language
/// server, survives the kept cancel and is the same process the next turn finds; the turn's
/// `setsid` job beside it dies.
#[tokio::test(flavor = "multi_thread")]
async fn the_agents_own_helper_survives_a_kept_cancel() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[helper] [background 3000] [hold]", CANCEL);
    let helper = pinned(&fixture.checkout(), "helper.pid").await;
    let job = pinned(&fixture.checkout(), "background.pid").await;
    let pid = std::fs::read_to_string(fixture.checkout().join("helper.pid")).unwrap();
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let at_settlement = settled_with(&fixture, "cancel-1", &job).await;
    let helper_alive = !helper.has_exited().unwrap();
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "[helper]")),
    );
    turns_ended(&fixture, 2).await;
    let reused = answered(&fixture, &format!("helper {} running", pid.trim()));
    assert_eq!(
        (
            at_settlement,
            helper_alive,
            reused,
            end(&fixture, &sessions).await
        ),
        (
            (TurnCancelOutcome::Stopped, false),
            true,
            true,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Cases 2 and 6: a turn that ends normally kills nothing, and a `setsid` job it left predates the next turn,
/// so that turn's kept cancel leaves it running too. Only the end of the session kills it.
#[tokio::test(flavor = "multi_thread")]
async fn jobs_of_a_finished_turn_survive_the_next_turns_cancel() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    let began = Instant::now();
    fixture.start_keeping(&sessions, "[background 8000] [helper] earlier", CANCEL);
    let job = pinned(&fixture.checkout(), "background.pid").await;
    let helper = pinned(&fixture.checkout(), "helper.pid").await;
    turns_ended(&fixture, 1).await;
    // Past the quiet window a stopped turn would have waited.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after_finish = (!job.has_exited().unwrap(), !helper.has_exited().unwrap());
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "[hold] later")),
    );
    prompted(&fixture, "later").await;
    fixture.command(
        &sessions,
        "cancel-2",
        SessionCommand::CancelTurn(TurnId::new("turn-2")),
    );
    let at_settlement = settled_with(&fixture, "cancel-2", &job).await;
    let reason = end(&fixture, &sessions).await;
    let killed_by_end = job.has_exited().unwrap();
    assert_never_wrote(&fixture.checkout(), began, Duration::from_millis(8000));
    assert_eq!(
        (after_finish, at_settlement, reason, killed_by_end),
        (
            (true, true),
            (TurnCancelOutcome::Stopped, true),
            AgentSessionEndReason::UserEnded,
            true
        )
    );
}

/// Cases 4 and 8: an Agent that keeps starting `setsid` jobs after it answered never lets two scans in a row
/// come back clean: past the fence's bound the session ends with `turn_processes_unstopped`
/// instead of being kept, and its stop kills the jobs that remain.
#[tokio::test(flavor = "multi_thread")]
async fn jobs_that_keep_coming_back_end_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_fence = Duration::from_secs(1);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[respawn] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let ended = fixture.ended().await;
    assert_eq!(
        (
            fixture.ledger.cancels(),
            ended.reason,
            ended.detail.as_deref()
        ),
        (
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            AgentSessionEndReason::AgentFailed,
            Some("turn_processes_unstopped")
        )
    );
}

/// Case 4: a fence that cannot tell the turn's processes from others, here because the attempt's mark
/// scope is gone, never keeps the session: it ends with `turn_processes_unstopped`. The scope is
/// put back once the end is decided, so the session's own stop can be proven.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_fence_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    let attempts = fixture.home().join("session-attempts");
    let scope = std::fs::read_dir(&attempts)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "mark")
        })
        .expect("the attempt recorded its mark scope");
    let recorded = std::fs::read(&scope).unwrap();
    std::fs::remove_file(&scope).unwrap();
    cancel(&fixture, &sessions, "cancel-1");
    until(|| fixture.ledger.decision()).await;
    std::fs::write(&scope, recorded).unwrap();
    let ended = fixture.ended().await;
    assert_eq!(
        (fixture.ledger.cancels(), ended.detail.as_deref()),
        (
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            Some("turn_processes_unstopped")
        )
    );
}

/// Case 5: a version listed only for another platform is not verified on this Node: the cancel ends the
/// session with `turn_boundary_unverified`, and the fence never runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_version_verified_only_on_another_platform_is_unverified() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.turn_boundary =
        vec![boundary(PLUGIN_VERSION, TurnBoundaryPlatform::Windows)];
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    assert_eq!(
        fixture.ended().await.detail.as_deref(),
        Some("turn_boundary_unverified")
    );
}

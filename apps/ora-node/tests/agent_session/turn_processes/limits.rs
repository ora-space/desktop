//! When the fence runs at all, and within which bounds: never for a stop that cannot keep the
//! session, also for a failed turn that may, and never past its budget.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md; each test names the case it
//! covers.
use super::*;
use pretty_assertions::assert_eq;
use std::sync::{Arc, Mutex, PoisonError};

const FAILURE: Keep = Keep {
    on_cancel: false,
    on_failure: true,
};

/// The one-letter state of a process, `None` once it is gone.
fn state(pid: u32) -> Option<char> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(") ")?
        .1
        .chars()
        .next()
}

/// Records, when the end decision is written and before the Agent is signalled, whether the
/// `[background]` job still runs. A fence would have killed it before the decision; the session's
/// stop kills it only after. The job records its PID as it starts, which the hook waits for.
fn alive_at_decision(fixture: &Fixture) -> Arc<Mutex<Option<bool>>> {
    let seen = Arc::new(Mutex::new(None));
    let (record, out) = (fixture.checkout().join("background.pid"), Arc::clone(&seen));
    fixture.ledger.on_decision(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(pid) = std::fs::read_to_string(&record)
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
            {
                break Some(pid);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let alive = pid.is_some_and(|pid| state(pid).is_some_and(|state| state != 'Z'));
        *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(alive);
    });
    seen
}

/// Cases 5 and 6: a stop that cannot keep the session ends it without a fence, so the turn's
/// job still runs when the end is decided: a failure the session did not choose to survive, a
/// cancel of a version not verified on this platform, and a cancel answered with a tool call
/// still open.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_that_cannot_keep_the_session_runs_no_fence() {
    for (prompt, keep, verified, detail) in [
        (
            "[background 3000] [request-failed]",
            CANCEL,
            true,
            "agent_turn_failed",
        ),
        (
            "[background 3000] [hold]",
            CANCEL,
            false,
            "turn_boundary_unverified",
        ),
        (
            "[background 3000] [hold-running-tool]",
            CANCEL,
            true,
            "turn_not_quiescent",
        ),
    ] {
        let mut fixture = Fixture::new();
        if verified {
            fixture.verify_turn_boundary();
        }
        let sessions = fixture.sessions(PLUGIN_VERSION);
        let alive = alive_at_decision(&fixture);
        fixture.start_keeping(&sessions, prompt, keep);
        if prompt.contains("[hold") {
            pinned(&fixture.checkout(), "background.pid").await;
            prompted(&fixture, "[hold").await;
            cancel(&fixture, &sessions, "cancel-1");
        }
        let ended = fixture.ended().await;
        let alive = *alive.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            (prompt, ended.detail.as_deref(), alive),
            (prompt, Some(detail), Some(true))
        );
    }
}

/// Case 5: a failed turn the session chose to survive runs the fence too: its `setsid` job dies
/// before the next turn and never writes, and the session takes the next turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_the_session_survives_runs_the_fence() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    let began = Instant::now();
    fixture.start_keeping(&sessions, "[background 3000] [request-failed]", FAILURE);
    turns_ended(&fixture, 1).await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
    );
    turns_ended(&fixture, 2).await;
    // The fence may have killed the job before it recorded its PID.
    let job_alive = std::fs::read_to_string(fixture.checkout().join("background.pid"))
        .ok()
        .and_then(|pid| pid.trim().parse::<u32>().ok())
        .is_some_and(|pid| state(pid).is_some_and(|state| state != 'Z'));
    let kept = fixture.ledger.ended().is_none();
    assert_never_wrote(&fixture.checkout(), began, Duration::from_millis(3000));
    assert_eq!(
        (job_alive, kept, end(&fixture, &sessions).await),
        (false, true, AgentSessionEndReason::UserEnded)
    );
}

/// Cases 4 and 5: a failed turn whose fence never gets clean ends the session with
/// `turn_processes_unstopped` within F of the answer: the bound starts with the fence, not with
/// any cancel, and nothing waits for Q or R first.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_fence_ends_within_its_bound() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_fence = Duration::from_secs(1);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    let decided = Arc::new(Mutex::new(None));
    let at = Arc::clone(&decided);
    fixture.ledger.on_decision(move || {
        *at.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    });
    fixture.start_keeping(&sessions, "[respawn] [request-failed]", FAILURE);
    turns_ended(&fixture, 1).await;
    let answered = Instant::now();
    let ended = fixture.ended().await;
    let decided = decided
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .expect("the end was decided");
    // F plus slack for a busy machine; Q (300 ms) and R are not part of this end.
    let within = decided.saturating_duration_since(answered) < Duration::from_secs(3);
    assert_eq!(
        (ended.detail.as_deref(), within),
        (Some("turn_processes_unstopped"), true)
    );
}

/// Case 4: a fence whose budget F is spent before its first observation could start ends the
/// session with `turn_processes_unstopped`, although the turn left nothing running.
#[tokio::test(flavor = "multi_thread")]
async fn a_fence_past_its_budget_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_fence = Duration::ZERO;
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let ended = fixture.ended().await;
    assert_eq!(
        (fixture.ledger.cancels(), ended.detail.as_deref()),
        (
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            Some("turn_processes_unstopped")
        )
    );
}

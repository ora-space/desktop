//! The turn process fence against real processes started through an attempt's spawn gate.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md cases 1, 4 and 5.
use super::marks::detached_job;
use super::{TestResult, fixture, journal};
use crate::{
    managed::attempt_lock::StopEvidence,
    session::stop::{
        self,
        turn::{TurnProcesses, TurnStart, TurnStop, observe_until},
    },
};
use ora_node_db::{AttemptId, SessionStopIdentity};
use ora_node_protocol::ExecutionId;
use ora_process::TokioProcessSpawner;
use pretty_assertions::assert_eq;
use std::time::Duration;

const UNKNOWN: TurnStop = TurnStop::Unstopped {
    evidence: StopEvidence::Unknown,
};

/// An attempt registered and opened the way a session's is.
fn attempt(
    journal: &ora_node_db::SessionJournal,
    home: &std::path::Path,
    identity: SessionStopIdentity,
) -> Result<stop::spawner::SessionAttemptSpawner, Box<dyn std::error::Error>> {
    Ok(stop::prepare(
        journal,
        home,
        identity,
        TokioProcessSpawner::new().without_reaper_registration(),
    )?)
}

/// A boot clock that cannot be read.
fn broken_clock() -> std::io::Result<u64> {
    Err(std::io::Error::other("injected clock failure"))
}

/// Case 4: a turn whose start could not be recorded is never reported stopped, by the fence or
/// by the re-check, and its job is not signalled.
#[tokio::test]
async fn a_turn_without_a_recorded_start_is_never_stopped() -> TestResult {
    let (home, _, identity) = fixture()?;
    let spawner = attempt(&journal(home.path())?, home.path(), identity)?;
    let processes = TurnProcesses::with_clock(spawner.clone(), broken_clock);
    let start = processes.start();
    let (job, _) = detached_job(&spawner).await?;
    let execution = ExecutionId::new("session");
    let fenced = processes
        .stop(&execution, Some(&start), Duration::from_secs(1))
        .await;
    let rechecked = processes.recheck(&execution, Some(&start)).await;
    let signalled = job.has_exited()?;
    job.signal(ora_utils::process::ProcessSignal::Kill)?;
    assert_eq!(
        (start, fenced, rechecked, signalled),
        (TurnStart::Unrecorded, UNKNOWN, UNKNOWN, false)
    );
    Ok(())
}

/// Case 4: when MCP lifecycle replaced the attempt during the turn, the turn's start names the
/// old one; the fence reports Unknown without signalling anything.
#[tokio::test]
async fn an_attempt_replaced_during_the_turn_is_never_stopped() -> TestResult {
    let (home, _, identity) = fixture()?;
    let journal = journal(home.path())?;
    let spawner = attempt(&journal, home.path(), identity.clone())?;
    let processes = TurnProcesses::new(spawner.clone());
    let start = processes.start();
    let (job, _) = detached_job(&spawner).await?;
    let next = attempt(
        &journal,
        home.path(),
        SessionStopIdentity {
            attempt: AttemptId::generate(),
            ..identity
        },
    )?;
    spawner.replace(next);
    let execution = ExecutionId::new("session");
    let fenced = processes
        .stop(&execution, Some(&start), Duration::from_secs(1))
        .await;
    let signalled = job.has_exited()?;
    job.signal(ora_utils::process::ProcessSignal::Kill)?;
    assert_eq!((fenced, signalled), (UNKNOWN, false));
    Ok(())
}

/// Cases 1 and 5: the fence kills a `setsid` job the turn started; the scan-only re-check finds
/// one started after the fence came back clean and leaves it running.
#[tokio::test]
async fn the_recheck_finds_a_job_started_after_a_clean_fence_and_kills_nothing() -> TestResult {
    let (home, _, identity) = fixture()?;
    let spawner = attempt(&journal(home.path())?, home.path(), identity)?;
    let processes = TurnProcesses::new(spawner.clone());
    // Start times have clock-tick granularity: the spawner's own start must fall before the turn.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let start = processes.start();
    let (early, _) = detached_job(&spawner).await?;
    let execution = ExecutionId::new("session");
    let fenced = processes
        .stop(&execution, Some(&start), Duration::from_secs(5))
        .await;
    let early_gone = early.has_exited()?;
    let clean = processes.recheck(&execution, Some(&start)).await;
    let (late, _) = detached_job(&spawner).await?;
    let rechecked = processes.recheck(&execution, Some(&start)).await;
    let late_alive = !late.has_exited()?;
    late.signal(ora_utils::process::ProcessSignal::Kill)?;
    assert_eq!(
        (fenced, early_gone, clean, rechecked, late_alive),
        (
            TurnStop::Stopped,
            true,
            TurnStop::Stopped,
            TurnStop::Unstopped {
                evidence: StopEvidence::Running
            },
            true
        )
    );
    Ok(())
}

/// Case 4: an observation that is not clean is repeated after the shared backoff while another
/// one fits before the deadline; a clean one ends the wait, and one that cannot succeed by
/// repetition ends it as Unknown at once.
#[tokio::test]
async fn observations_back_off_until_one_is_clean_or_the_bound_passes() {
    /// Runs `observe_until` over recorded outcomes and counts the observations.
    async fn run(outcomes: Vec<Option<StopEvidence>>, bound: Duration) -> (StopEvidence, usize) {
        let mut outcomes = outcomes.into_iter();
        let mut calls = 0;
        let evidence = observe_until(tokio::time::Instant::now() + bound, || {
            calls += 1;
            let next = outcomes.next().unwrap_or(Some(StopEvidence::Running));
            async move { next }
        })
        .await;
        (evidence, calls)
    }
    let running = Some(StopEvidence::Running);
    assert_eq!(
        [
            run(
                vec![
                    running,
                    Some(StopEvidence::Unknown),
                    Some(StopEvidence::Stopped)
                ],
                Duration::from_secs(5)
            )
            .await,
            // 0, 25 and 75 ms: a fourth observation would not fit before 100 ms.
            run(Vec::new(), Duration::from_millis(100)).await,
            run(vec![None, running], Duration::from_secs(5)).await,
        ],
        [
            (StopEvidence::Stopped, 3),
            (StopEvidence::Running, 3),
            (StopEvidence::Unknown, 1)
        ]
    );
}

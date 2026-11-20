//! A job that left every original group and closed the inherited lock stays the attempt's.
use super::{TestResult, fixture, journal};
use crate::{managed::attempt_lock::StopEvidence, session::stop};
use ora_node_db::{SessionStopProof, SessionStopScope, SessionStopState};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, TokioProcessSpawner};
use ora_utils::process::{ClaimedProcess, LinuxPidFd, MarkScan, linux_process};
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

/// The direct child forks a job that closes every inherited descriptor, as Bun does, starts its
/// own session, and outlives its parent; it prints the job's PID once the job is ready.
const DETACHED_JOB: &str = "import os,sys,time
read,write=os.pipe()
if os.fork():
    os.close(write)
    os.read(read,1)
    sys.exit(0)
os.close(read)
for name in os.listdir('/proc/self/fd'):
    fd=int(name)
    if fd>2 and fd!=write:
        try: os.close(fd)
        except OSError: pass
os.setsid()
print(os.getpid(),flush=True)
os.close(write)
time.sleep(60)";

/// Starts the job through the attempt's own spawn gate and returns it pinned.
async fn detached_job(
    spawner: &stop::spawner::SessionAttemptSpawner,
) -> Result<LinuxPidFd, Box<dyn std::error::Error>> {
    let mut launcher = spawner.spawn(ProcessSpec::new("python3").args(["-c", DETACHED_JOB]))?;
    let output = launcher.take_stdout().ok_or("missing stdout")?;
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(output).read_line(&mut line),
    )
    .await??;
    let job = LinuxPidFd::from_observation(&linux_process(line.trim().parse()?)?)?;
    launcher.wait().await?;
    Ok(job)
}

/// Re-probes until the outcome is `expected` or two seconds pass, and returns the last outcome.
///
/// Concurrent tests fork while this one runs; until their children exec, they briefly hold a
/// copy of every inherited descriptor, the attempt lock included, so the lock layer can read
/// Running for a moment. Waiting it out keeps each assertion about the layer under test.
async fn eventually(
    probe: impl Fn() -> (StopEvidence, Option<SessionStopProof>),
    expected: (StopEvidence, Option<SessionStopProof>),
) -> (StopEvidence, Option<SessionStopProof>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let observed = probe();
        if observed == expected || tokio::time::Instant::now() >= deadline {
            return observed;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The path of an attempt's mark scope record.
fn scope_path(home: &std::path::Path, attempt: &ora_node_db::AttemptId) -> std::path::PathBuf {
    home.join("session-attempts")
        .join(format!("{attempt}.mark"))
}

/// Lock and clues alone would call this attempt stopped; its mark keeps it running until the
/// terminate path kills the job, and only then is the attempt proof.
#[tokio::test]
async fn a_detached_job_blocks_proof_until_terminate_kills_it() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    let spawner = stop::prepare(
        &journal,
        home.path(),
        identity.clone(),
        TokioProcessSpawner::new().without_reaper_registration(),
    )?;
    let job = detached_job(&spawner).await?;
    spawner.close();
    let probe = || files.probe(&identity, SessionStopScope::Recorded, &identity.host, false);
    let held = probe();
    files.terminate(&identity, &identity.host);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !job.has_exited().unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let stopped = (
        StopEvidence::Stopped,
        Some(SessionStopProof::LockProcessesAndMarks),
    );
    assert_eq!(
        (held, eventually(probe, stopped).await),
        ((StopEvidence::Running, None), stopped)
    );
    Ok(())
}

/// Evidence follows observation, never a kill request: a marked job that survives (as one the
/// Node may not signal would) stays running, and a damaged scope proves nothing.
#[tokio::test]
async fn a_surviving_job_or_damaged_scope_is_never_proof() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    let spawner = stop::prepare(
        &journal,
        home.path(),
        identity.clone(),
        TokioProcessSpawner::new().without_reaper_registration(),
    )?;
    let job = detached_job(&spawner).await?;
    spawner.close();
    let probe = || files.probe(&identity, SessionStopScope::Recorded, &identity.host, false);
    let surviving = eventually(probe, (StopEvidence::Running, None)).await;
    std::fs::write(scope_path(home.path(), &identity.attempt), b"damaged")?;
    let damaged = eventually(probe, (StopEvidence::Unknown, None)).await;
    job.signal(ora_utils::process::ProcessSignal::Kill)?;
    assert_eq!(
        (surviving, damaged),
        ((StopEvidence::Running, None), (StopEvidence::Unknown, None))
    );
    Ok(())
}

/// The ledger says this attempt was registered with a scope, so a lost scope record is missing
/// evidence: Unknown even though the lock and clues are clean, never the weaker proof.
#[tokio::test]
async fn a_scoped_attempt_whose_scope_is_lost_is_unknown() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    let spawner = stop::prepare(
        &journal,
        home.path(),
        identity.clone(),
        TokioProcessSpawner::new().without_reaper_registration(),
    )?;
    spawner.close();
    std::fs::remove_file(scope_path(home.path(), &identity.attempt))?;
    let probe = || files.probe(&identity, SessionStopScope::Recorded, &identity.host, false);
    let config = crate::SessionStopConfig {
        confirmation_timeout_ms: 200,
        ..Default::default()
    };
    let execution = ora_node_protocol::ExecutionId::new("session");
    let confirmed = stop::confirm(&journal, home.path(), &config, &execution, true).await?;
    let [record] = journal
        .session_stop_records(Some(&execution))?
        .try_into()
        .unwrap();
    assert_eq!(
        (
            eventually(probe, (StopEvidence::Unknown, None)).await,
            confirmed,
            record.scope,
            record.state,
            record.proof
        ),
        (
            (StopEvidence::Unknown, None),
            false,
            SessionStopScope::Recorded,
            SessionStopState::StopUnconfirmed,
            None
        )
    );
    Ok(())
}

/// Confirms every attempt of the fixture execution through the production owner and returns the
/// ledger record of `attempt`.
async fn confirmed(
    journal: &ora_node_db::SessionJournal,
    home: &std::path::Path,
    attempt: &ora_node_db::AttemptId,
) -> Result<ora_node_db::SessionStopRecord, Box<dyn std::error::Error>> {
    let config = crate::SessionStopConfig::default();
    let execution = ora_node_protocol::ExecutionId::new("session");
    assert!(stop::confirm(journal, home, &config, &execution, true).await?);
    Ok(journal
        .session_stop_records(Some(&execution))?
        .into_iter()
        .find(|record| &record.identity.attempt == attempt)
        .ok_or("missing attempt")?)
}

/// An attempt with a mark scope is confirmed only after the scan, and the ledger says so.
#[tokio::test]
async fn a_marked_attempt_records_lock_processes_and_marks() -> TestResult {
    let (home, _files, identity) = fixture()?;
    let journal = journal(home.path())?;
    let spawner = stop::prepare(
        &journal,
        home.path(),
        identity.clone(),
        TokioProcessSpawner::new().without_reaper_registration(),
    )?;
    let job = detached_job(&spawner).await?;
    spawner.close();
    let record = confirmed(&journal, home.path(), &identity.attempt).await?;
    assert_eq!(
        (
            job.has_exited()?,
            record.state,
            record.proof,
            record.scope,
            record.confirmed_at_ms.is_some()
        ),
        (
            true,
            SessionStopState::Confirmed,
            Some(SessionStopProof::LockProcessesAndMarks),
            SessionStopScope::Recorded,
            true
        )
    );
    Ok(())
}

/// An attempt an older Node registered carries no scope flag and has no `.mark`; it keeps the
/// lock-and-clue evidence it had.
#[tokio::test]
async fn an_unscoped_attempt_keeps_lock_and_processes() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    files.create(&identity.attempt, ora_process::ProcessIdentity::Inherit)?;
    // The on-disk state an older Node leaves: lock and clues, but no `.mark` scope.
    std::fs::remove_file(scope_path(home.path(), &identity.attempt))?;
    journal.record_session_stop(&identity, SessionStopScope::Absent, stop::now_ms())?;
    let record = confirmed(&journal, home.path(), &identity.attempt).await?;
    assert_eq!(
        (record.state, record.proof, record.scope),
        (
            SessionStopState::Confirmed,
            Some(SessionStopProof::LockAndProcesses),
            SessionStopScope::Absent
        )
    );
    Ok(())
}

/// Feeds recorded scan outcomes, in order, to the evidence rule.
fn judged(scans: Vec<std::io::Result<MarkScan>>) -> StopEvidence {
    let mut scans = scans.into_iter();
    stop::marks::evidence(|| {
        scans
            .next()
            .unwrap_or_else(|| Err(std::io::Error::other("no further scan")))
    })
}

/// Stopped needs two clean scans in a row; a claimed process means Running, and an undecided
/// process or a failed scan means Unknown, whichever scan shows it.
#[test]
fn evidence_needs_two_clean_scans_without_undecided_processes() {
    let clean = || Ok(MarkScan::default());
    let claimed = || {
        Ok(MarkScan {
            claimed: vec![ClaimedProcess {
                pid: 7,
                start_ticks: 1,
            }],
            undecided: 0,
        })
    };
    let undecided = || {
        Ok(MarkScan {
            claimed: vec![],
            undecided: 1,
        })
    };
    assert_eq!(
        [
            judged(vec![clean(), clean()]),
            judged(vec![clean(), claimed()]),
            judged(vec![claimed()]),
            judged(vec![undecided()]),
            judged(vec![clean(), undecided()]),
            judged(vec![Err(std::io::Error::other("procfs"))]),
        ],
        [
            StopEvidence::Stopped,
            StopEvidence::Running,
            StopEvidence::Running,
            StopEvidence::Unknown,
            StopEvidence::Unknown,
            StopEvidence::Unknown,
        ]
    );
}

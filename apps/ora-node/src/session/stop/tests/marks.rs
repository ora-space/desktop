//! A job that left every original group and closed the inherited lock stays the attempt's.
use super::{TestResult, fixture, journal};
use crate::{managed::attempt_lock::StopEvidence, session::stop};
use ora_node_db::SessionStopProof;
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, TokioProcessSpawner};
use ora_utils::process::{LinuxPidFd, linux_process};
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
    let held = files.probe(&identity, &identity.host, false);
    files.terminate(&identity, &identity.host);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !job.has_exited().unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert_eq!(
        (held, files.probe(&identity, &identity.host, false)),
        (
            (StopEvidence::Running, None),
            (
                StopEvidence::Stopped,
                Some(SessionStopProof::LockProcessesAndMarks)
            )
        )
    );
    Ok(())
}

/// Evidence follows observation, never a kill request: a marked job that survives (as one the
/// Node may not signal would) stays running, and a damaged scope proves nothing. An attempt
/// recorded before marks existed keeps its lock-and-clue evidence.
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
    let scope = home
        .path()
        .join("session-attempts")
        .join(format!("{}.mark", identity.attempt));
    let surviving = files.probe(&identity, &identity.host, false);
    let original = std::fs::read(&scope)?;
    std::fs::write(&scope, b"damaged")?;
    let damaged = files.probe(&identity, &identity.host, false);
    std::fs::remove_file(&scope)?;
    let legacy = files.probe(&identity, &identity.host, false);
    std::fs::write(&scope, original)?;
    job.signal(ora_utils::process::ProcessSignal::Kill)?;
    assert_eq!(
        (surviving, damaged, legacy),
        (
            (StopEvidence::Running, None),
            (StopEvidence::Unknown, None),
            (
                StopEvidence::Stopped,
                Some(SessionStopProof::LockAndProcesses)
            )
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
            record.confirmed_at_ms.is_some()
        ),
        (
            true,
            ora_node_db::SessionStopState::Confirmed,
            Some(SessionStopProof::LockProcessesAndMarks),
            true
        )
    );
    Ok(())
}

/// An attempt an older Node registered has no mark scope; it keeps the evidence it had.
#[tokio::test]
async fn a_pre_mark_attempt_keeps_lock_and_processes() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    files.create(&identity.attempt, ora_process::ProcessIdentity::Inherit)?;
    // The on-disk state an older Node leaves: lock and clues, but no `.mark` scope.
    std::fs::remove_file(
        home.path()
            .join("session-attempts")
            .join(format!("{}.mark", identity.attempt)),
    )?;
    journal.record_session_stop(&identity, stop::now_ms())?;
    let record = confirmed(&journal, home.path(), &identity.attempt).await?;
    assert_eq!(
        (record.state, record.proof),
        (
            ora_node_db::SessionStopState::Confirmed,
            Some(SessionStopProof::LockAndProcesses)
        )
    );
    Ok(())
}

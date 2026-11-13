//! Stop evidence against real processes: what holds an attempt lock, and what a clue may kill.
use super::attempt_lock::*;
use ora_node_db::AttemptId;
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, TokioProcessSpawner};
use pretty_assertions::assert_eq;
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, BufReader};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A lock directory under a private temporary Node home.
fn locks() -> Result<(tempfile::TempDir, AttemptLocks), Box<dyn std::error::Error>> {
    let home = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir()?;
    let locks = AttemptLocks::open(home.path())?;
    Ok((home, locks))
}

/// Creates and takes an attempt's lock, then starts `script` with it and gives the lock up, as
/// the Node does for a command it dispatches.
fn start(
    locks: &AttemptLocks,
    attempt: &AttemptId,
    script: &str,
) -> Result<ora_process::TokioManagedProcess, Box<dyn std::error::Error>> {
    locks.create(attempt, Path::new("/"), LockDurability::Unrecorded)?;
    let descriptor = locks.hold(attempt)?;
    Ok(TokioProcessSpawner::new().spawn(
        ProcessSpec::new("/bin/sh")
            .args(["-c", script])
            .inherit_descriptor(descriptor),
    )?)
}

/// Polls the lock until it reports `expected` or five seconds pass.
async fn eventually(locks: &AttemptLocks, attempt: &AttemptId, expected: StopEvidence) -> bool {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while locks.probe(attempt) != expected {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(/*millis*/ 10)).await;
    }
    true
}

/// The lock directory and files are private to the Node, and a lock file that does not exist is
/// missing evidence, never a stopped command.
#[test]
fn lock_files_are_private_and_a_missing_one_is_unknown() -> TestResult {
    let (home, locks) = locks()?;
    let attempt = AttemptId::generate();
    let before = locks.probe(&attempt);
    locks.create(&attempt, Path::new("/checkout"), LockDurability::Recorded)?;
    let directory = home.path().join("git-attempts");
    let modes = (
        std::fs::metadata(&directory)?.permissions().mode() & 0o777,
        std::fs::metadata(directory.join(format!("{attempt}.lock")))?
            .permissions()
            .mode()
            & 0o777,
    );
    assert_eq!(
        (before, modes, locks.probe(&attempt), locks.entries()?),
        (
            StopEvidence::Unknown,
            (0o700, 0o600),
            StopEvidence::Stopped,
            vec![(attempt.clone(), "/checkout".into())]
        )
    );
    locks.retire(&attempt);
    assert_eq!(
        (locks.probe(&attempt), locks.entries()?),
        (StopEvidence::Unknown, vec![])
    );
    Ok(())
}

/// A descendant that left the command's process group and session still holds the lock. No clue
/// was written, as when the Node dies right after the spawn: the evidence does not need one. It
/// reads Running while the descendant lives and Stopped once it is gone.
#[tokio::test]
async fn a_descendant_in_another_session_keeps_the_lock_until_it_ends() -> TestResult {
    let (_home, locks) = locks()?;
    let attempt = AttemptId::generate();
    let mut command = start(
        &locks,
        &attempt,
        "setsid sleep 60 >/dev/null 2>&1 & echo $!",
    )?;
    let stdout = command.take_stdout().ok_or("stdout pipe")?;
    let escaped: i32 = BufReader::new(stdout)
        .lines()
        .next_line()
        .await?
        .ok_or("the script prints the descendant's pid")?
        .parse()?;
    command.wait().await?;
    // The direct child is gone and its group is empty; ending the group changes nothing.
    locks.terminate(&attempt);
    let while_running = locks.probe(&attempt);
    // SAFETY: the pid names the sleeper this test started in its own session.
    unsafe { libc::kill(escaped, libc::SIGKILL) };

    assert_eq!(
        (
            while_running,
            eventually(&locks, &attempt, StopEvidence::Stopped).await
        ),
        (StopEvidence::Running, true)
    );
    Ok(())
}

/// A clue only kills what it still identifies. A start time that does not match means the pid
/// now belongs to another process, and a clue from another boot names nothing here: neither is
/// signalled. The matching clue ends the group.
#[tokio::test]
async fn a_clue_that_no_longer_identifies_its_process_kills_nothing() -> TestResult {
    let (home, locks) = locks()?;
    let attempt = AttemptId::generate();
    let command = start(&locks, &attempt, "exec sleep 60")?;
    let pid = command.id().ok_or("pid")?;
    locks.note_child(&attempt, pid);
    let clue = home
        .path()
        .join("git-attempts")
        .join(format!("{attempt}.clue"));
    let written = std::fs::read_to_string(&clue)?;
    let fields: Vec<&str> = written.split(' ').collect();

    let mut survived = Vec::new();
    for stale in [
        format!("{pid} {} {}", fields[1].parse::<u64>()? + 1, fields[2]),
        format!("{pid} {} another-boot", fields[1]),
    ] {
        std::fs::write(&clue, stale)?;
        locks.terminate(&attempt);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 100)).await;
        survived.push(locks.probe(&attempt));
    }
    std::fs::write(&clue, written)?;
    locks.terminate(&attempt);
    command.wait().await?;

    assert_eq!(
        (
            survived,
            eventually(&locks, &attempt, StopEvidence::Stopped).await
        ),
        (vec![StopEvidence::Running, StopEvidence::Running], true)
    );
    Ok(())
}

/// When the group leader has exited, its pid cannot be reused while the group still has members,
/// so the clue ends the members that stayed behind.
#[tokio::test]
async fn a_clue_ends_the_members_its_exited_leader_left() -> TestResult {
    let (_home, locks) = locks()?;
    let attempt = AttemptId::generate();
    let mut command = start(
        &locks,
        &attempt,
        "sleep 60 >/dev/null 2>&1 & echo started; read wait_for_the_clue",
    )?;
    let mut stdin = command.take_stdin().ok_or("stdin pipe")?;
    let stdout = command.take_stdout().ok_or("stdout pipe")?;
    BufReader::new(stdout).lines().next_line().await?;
    locks.note_child(&attempt, command.id().ok_or("pid")?);
    // Closing stdin lets the leader exit while its background sleeper keeps the group alive.
    tokio::io::AsyncWriteExt::shutdown(&mut stdin).await?;
    drop(stdin);
    command.wait().await?;
    let after_leader_exit = locks.probe(&attempt);

    assert_eq!(
        (
            after_leader_exit,
            locks
                .await_stop(&attempt, Duration::from_secs(/*secs*/ 5))
                .await
        ),
        (StopEvidence::Running, StopEvidence::Stopped)
    );
    Ok(())
}

/// A leftover lock blocks until its command is gone, then retires itself.
#[tokio::test]
async fn a_leftover_blocks_until_its_command_is_gone() -> TestResult {
    let (_home, locks) = locks()?;
    let attempt = AttemptId::generate();
    let command = start(&locks, &attempt, "exec sleep 60")?;
    locks.keep_leftover(attempt.clone(), "/transport".into());
    let blocked = locks.leftover_directories();
    command.kill().await?;
    command.wait().await?;

    assert_eq!(
        (blocked, locks.leftover_directories(), locks.entries()?),
        (vec!["/transport".into()], vec![], vec![])
    );
    Ok(())
}

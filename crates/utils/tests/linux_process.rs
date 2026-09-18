#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process_snapshot};
use pretty_assertions::assert_eq;

/// Stable handles preserve exit evidence, permit peeking, and do not signal a subsequent child.
#[test]
fn pidfd_observation_signaling_and_reaping() -> Result<(), Box<dyn std::error::Error>> {
    LinuxPidFd::probe_current()?;
    let child = Command::new("/bin/sleep").arg("60").spawn()?;
    let handle = LinuxPidFd::for_child(&child)?;
    let observed = linux_process_snapshot()?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|stat| stat.pid == child.id())
        .ok_or("missing child observation")?;
    let pinned = LinuxPidFd::from_observation(&observed)?;
    assert_eq!(handle.has_exited()?, false);
    pinned.signal(ProcessSignal::Kill)?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !handle.has_exited()? {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(/*millis*/ 1));
    }
    for _ in 0..2 {
        assert_eq!(
            handle.peek_child_exit()?.and_then(|status| status.signal()),
            Some(libc::SIGKILL)
        );
    }
    assert_eq!(handle.reap_child()?.signal(), Some(libc::SIGKILL));
    assert!(LinuxPidFd::from_observation(&observed).is_err());
    let next = Command::new("/bin/sleep").arg("60").spawn()?;
    let next_handle = LinuxPidFd::for_child(&next)?;
    pinned.signal(ProcessSignal::Kill)?;
    assert_eq!(next_handle.has_exited()?, false);
    next_handle.signal(ProcessSignal::Kill)?;
    next_handle.reap_child()?;
    Ok(())
}

/// A process-controlled name cannot break the stat field layout used for identity discovery.
#[test]
fn proc_stat_accepts_non_utf8_names_and_parentheses() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "proc_name_fixture", "--nocapture"])
        .env("ORA_UTILS_PROC_FIXTURE", "1")
        .stdout(Stdio::piped())
        .spawn()?;
    let handle = LinuxPidFd::for_child(&child)?;
    let output = child.stdout.take().ok_or("stdout")?;
    let mut reader = BufReader::new(output);
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line)? > 0,
            "fixture exited without readiness"
        );
        if line.contains("PROC_READY") {
            break;
        }
    }
    let observed = linux_process_snapshot()?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|stat| stat.pid == child.id())
        .ok_or("missing named process")?;
    LinuxPidFd::from_observation(&observed)?.signal(ProcessSignal::Kill)?;
    assert_eq!(handle.reap_child()?.signal(), Some(libc::SIGKILL));
    Ok(())
}

/// The environment flag is passed only to this isolated child, never set in the test runner.
#[test]
fn proc_name_fixture() {
    if std::env::var_os("ORA_UTILS_PROC_FIXTURE").is_none() {
        return;
    }
    // /proc/<pid>/stat names the main thread, whereas this harness runs the test on a worker.
    std::fs::write("/proc/self/comm", b"child ) ( \xff")
        .unwrap_or_else(|error| panic!("comm: {error}"));
    println!("PROC_READY");
    std::thread::sleep(Duration::from_secs(/*secs*/ 20));
}

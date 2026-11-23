#![cfg(target_os = "linux")]
//! Real processes show which ones a mark claims, and that killing reaches jobs that left their
//! parent's process group or cleared their environment.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use ora_utils::process::{
    EnvironmentMark, LinuxPidFd, MarkScope, ProcessSignal, linux_boot_ticks, linux_process,
};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const NAME: &str = "ORA_UTILS_TEST_MARK";

/// A token no other test or process on the machine carries.
fn token(test: &str) -> std::io::Result<String> {
    Ok(format!(
        "{test}-{}-{}",
        std::process::id(),
        linux_boot_ticks()?
    ))
}

/// The scope a launch that starts now, as this test's user, would record.
fn scope(mark: &EnvironmentMark) -> Result<MarkScope, Box<dyn std::error::Error>> {
    // SAFETY: geteuid only reads this process's credentials.
    let uid = unsafe { libc::geteuid() };
    Ok(MarkScope::new(mark.clone(), linux_boot_ticks()?, uid))
}

/// Starts a long sleep, optionally with one value for the test mark.
fn sleeper(value: Option<&str>) -> std::io::Result<Child> {
    let mut command = Command::new("/bin/sleep");
    command.arg("60");
    if let Some(value) = value {
        command.env(NAME, value);
    }
    command.spawn()
}

/// Reads the PIDs a shell fixture echoes, one per line.
fn pids(stdout: ChildStdout, count: usize) -> std::io::Result<Vec<u32>> {
    BufReader::new(stdout)
        .lines()
        .take(count)
        .map(|line| {
            line?
                .trim()
                .parse()
                .map_err(|_| std::io::Error::other("fixture printed no pid"))
        })
        .collect()
}

/// Waits until every pinned process has exited.
fn exited(handles: &[LinuxPidFd]) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !handles
        .iter()
        .map(LinuxPidFd::has_exited)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .all(|exited| exited)
    {
        assert!(Instant::now() < deadline, "a claimed process survived");
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
    Ok(())
}

/// Pins a live PID for an exit check that cannot be confused by reuse.
fn pin(pid: u32) -> std::io::Result<LinuxPidFd> {
    LinuxPidFd::from_observation(&linux_process(pid)?)
}

/// Only processes with the exact token that started within the scope are claimed.
#[test]
fn scan_claims_only_marked_processes_started_within_the_scope() -> TestResult {
    let token = token("scope")?;
    let mark = EnvironmentMark::new(NAME, &token)?;
    let mut earlier = sleeper(Some(&token))?;
    // Start times have clock-tick granularity; wait several ticks before the scope begins.
    std::thread::sleep(Duration::from_millis(/*millis*/ 60));
    let scope = scope(&mark)?;
    let mut children = [
        sleeper(Some(&token))?,
        sleeper(Some(&format!("outer:{token}")))?,
        sleeper(Some(&format!("{token}-other")))?,
        sleeper(Some("another-launch"))?,
        sleeper(None)?,
    ];
    let mut claimed: Vec<_> = scope
        .scan()?
        .claimed
        .iter()
        .map(|process| process.pid)
        .collect();
    claimed.sort_unstable();
    let expected = vec![children[0].id(), children[1].id()];
    for child in children.iter_mut().chain([&mut earlier]) {
        child.kill()?;
        child.wait()?;
    }
    assert_eq!(claimed, expected);
    Ok(())
}

/// Killing reaches an orphaned job and an environment-cleared child of a live claimed parent,
/// and leaves an unrelated process of the same user running.
#[test]
fn kill_reaches_orphans_and_env_cleared_children_but_not_strangers() -> TestResult {
    let token = token("kill")?;
    let mark = EnvironmentMark::new(NAME, &token)?;
    let scope = scope(&mark)?;
    // The first shell exits at once, leaving its background job to init or a subreaper.
    let mut orphaning = Command::new("/bin/sh")
        .args(["-c", "/bin/sleep 60 & echo $!"])
        .env(NAME, &token)
        .stdout(Stdio::piped())
        .spawn()?;
    let orphan = pids(orphaning.stdout.take().ok_or("stdout")?, 1)?;
    orphaning.wait()?;
    // The second shell stays to wait for a child that dropped the whole environment.
    let mut parent = Command::new("/bin/sh")
        .args(["-c", "env -i /bin/sleep 60 & echo $!; wait"])
        .env(NAME, &token)
        .stdout(Stdio::piped())
        .spawn()?;
    let cleared = pids(parent.stdout.take().ok_or("stdout")?, 1)?;
    let mut stranger = sleeper(None)?;
    let handles = [pin(orphan[0])?, pin(cleared[0])?, pin(parent.id())?];
    scope.kill()?;
    exited(&handles)?;
    parent.wait()?;
    let survived = stranger.try_wait()?.is_none();
    stranger.kill()?;
    stranger.wait()?;
    assert!(survived, "an unmarked process was killed");
    assert!(scope.scan()?.claimed.is_empty());
    Ok(())
}

/// Starts a marked long sleep in a new session, as a tool shell that calls `setsid` would.
fn session_leader(value: &str) -> std::io::Result<Child> {
    let mut command = Command::new("/bin/sleep");
    command.arg("60").env(NAME, value);
    // SAFETY: setsid is async-signal-safe and only changes the forked child.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

/// Narrowed to new sessions since a tick, killing reaches only what started later outside the
/// sessions of earlier claimed processes: a later `setsid` job dies, while the earlier processes
/// and the later ones in their sessions keep running, the helper's child included.
#[test]
fn a_new_session_scope_kills_later_jobs_and_spares_earlier_sessions() -> TestResult {
    let token = token("sessions")?;
    let mark = EnvironmentMark::new(NAME, &token)?;
    let scope = scope(&mark)?;
    // Earlier: one process in this test's session, and a helper that leads its own session and
    // starts a child there once asked to.
    let mut earlier = sleeper(Some(&token))?;
    let mut helper = Command::new("/bin/sh");
    helper
        .args(["-c", "read line; /bin/sleep 60 & echo $!; wait"])
        .env(NAME, &token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    // SAFETY: setsid is async-signal-safe and only changes the forked child.
    unsafe {
        helper.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut helper = helper.spawn()?;
    // Start times have clock-tick granularity; wait several ticks before the later period.
    std::thread::sleep(Duration::from_millis(/*millis*/ 60));
    let later = scope.new_sessions_since(linux_boot_ticks()?);
    writeln!(helper.stdin.take().ok_or("stdin")?)?;
    let helper_child = pids(helper.stdout.take().ok_or("stdout")?, 1)?[0];
    let mut in_session = sleeper(Some(&token))?;
    let mut job = session_leader(&token)?;
    let spared = [
        pin(earlier.id())?,
        pin(helper.id())?,
        pin(helper_child)?,
        pin(in_session.id())?,
    ];
    let reported: Vec<_> = later
        .scan()?
        .claimed
        .iter()
        .map(|process| process.pid)
        .collect();
    later.kill()?;
    job.wait()?;
    let survived = spared
        .iter()
        .map(LinuxPidFd::has_exited)
        .collect::<Result<Vec<_>, _>>()?;
    let remaining = later.scan()?.claimed;
    scope.kill()?;
    for child in [&mut earlier, &mut helper, &mut in_session] {
        child.wait()?;
    }
    assert_eq!(
        (reported, survived, remaining),
        (vec![job.id()], vec![false; 4], Vec::new())
    );
    Ok(())
}

/// A process that made itself unreadable is still claimed and killed through its marked parent.
#[test]
fn an_unreadable_child_of_a_claimed_parent_is_killed_with_it() -> TestResult {
    let token = token("unreadable")?;
    let mark = EnvironmentMark::new(NAME, &token)?;
    let scope = scope(&mark)?;
    let exe = std::env::current_exe()?;
    let mut parent = Command::new("/bin/sh")
        .args([
            "-c",
            "\"$0\" --exact undumpable_fixture --nocapture >/dev/null & echo $!; wait",
        ])
        .arg(&exe)
        .env(NAME, &token)
        .env("ORA_UTILS_UNDUMPABLE_FIXTURE", "1")
        .stdout(Stdio::piped())
        .spawn()?;
    let child = pids(parent.stdout.take().ok_or("stdout")?, 1)?[0];
    let handle = pin(child)?;
    // The fixture turns non-dumpable after exec; only then is its environment unreadable, and
    // only to a scanner without CAP_SYS_PTRACE.
    // SAFETY: geteuid only reads this process's credentials.
    let root = unsafe { libc::geteuid() } == 0;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !root && std::fs::read(format!("/proc/{child}/environ")).is_ok() {
        assert!(Instant::now() < deadline, "fixture stayed dumpable");
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
    scope.kill()?;
    exited(&[handle])?;
    parent.wait()?;
    Ok(())
}

/// The environment flag reaches only the isolated child the test above starts.
#[test]
fn undumpable_fixture() {
    if std::env::var_os("ORA_UTILS_UNDUMPABLE_FIXTURE").is_none() {
        return;
    }
    // SAFETY: prctl with scalar arguments only changes this process's dumpable flag.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) }, 0);
    std::thread::sleep(Duration::from_secs(/*secs*/ 60));
}

/// Signals delivered through a pinned handle freeze before they kill.
#[test]
fn stop_freezes_a_pinned_process() -> TestResult {
    let mut child = sleeper(None)?;
    let handle = LinuxPidFd::for_child(&child)?;
    handle.signal(ProcessSignal::Stop)?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !std::fs::read_to_string(format!("/proc/{}/stat", child.id()))?.contains(") T ") {
        assert!(Instant::now() < deadline, "the process kept running");
        std::thread::sleep(Duration::from_millis(/*millis*/ 1));
    }
    handle.signal(ProcessSignal::Kill)?;
    child.wait()?;
    Ok(())
}

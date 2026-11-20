#![cfg(target_os = "linux")]
//! Real processes at the edges of a scan: more processes than descriptors, and a marked process
//! whose leader thread exited while its other threads keep running.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ora_utils::process::{EnvironmentMark, LinuxPidFd, MarkScope, linux_boot_ticks};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const NAME: &str = "ORA_UTILS_LIMIT_MARK";

/// More processes in the window than the fixture's lowered descriptor limit.
const CROWD: usize = 200;

/// The fixture's soft `RLIMIT_NOFILE` while it scans the crowd.
const LOWERED_LIMIT: u64 = 64;

/// A token no other test or process on the machine carries.
fn token(test: &str) -> std::io::Result<String> {
    Ok(format!(
        "{test}-{}-{}",
        std::process::id(),
        linux_boot_ticks()?
    ))
}

/// This test's own user, the user the scoped processes run as.
fn uid() -> u32 {
    // SAFETY: geteuid only reads this process's credentials.
    unsafe { libc::geteuid() }
}

/// Starts a long sleep, optionally carrying the test mark.
fn sleeper(value: Option<&str>) -> std::io::Result<Child> {
    let mut command = Command::new("/bin/sleep");
    command.arg("60");
    if let Some(value) = value {
        command.env(NAME, value);
    }
    command.spawn()
}

/// Kills and reaps its children however the test ends, so a failure leaves no process behind.
struct Reaped(Vec<Child>);

impl Drop for Reaped {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Waits until `ready` holds, failing after five seconds.
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !ready() {
        assert!(Instant::now() < deadline, "observation timed out");
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
}

/// A scan holds a bounded number of descriptors, so a window crowded beyond the soft limit is
/// still scanned and its marked process killed; with no descriptor left at all, the scan fails
/// instead of reporting a clean window.
#[test]
fn a_window_larger_than_the_descriptor_limit_is_still_scanned_and_killed() -> TestResult {
    let token = token("crowd")?;
    let since = linux_boot_ticks()?;
    let mut crowd = Reaped(
        (0..CROWD)
            .map(|_| sleeper(None))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let mut marked = Reaped(vec![sleeper(Some(&token))?]);
    let handle = LinuxPidFd::for_child(&marked.0[0])?;
    let marked_pid = marked.0[0].id();
    let mut fixture = Command::new(std::env::current_exe()?)
        .args(["--exact", "descriptor_limited_fixture", "--nocapture"])
        .env(
            "ORA_UTILS_LIMIT_FIXTURE",
            format!("{token} {since} {}", uid()),
        )
        .stdout(Stdio::piped())
        .spawn()?;
    let mut report = String::new();
    fixture
        .stdout
        .take()
        .ok_or("stdout")?
        .read_to_string(&mut report)?;
    fixture.wait()?;
    until(|| handle.has_exited().unwrap_or(false));
    marked.0[0].wait()?;
    let survivors = crowd
        .0
        .iter_mut()
        .map(|child| child.try_wait().map(|status| status.is_none()))
        .collect::<Result<Vec<_>, _>>()?;
    let lines: Vec<_> = report
        .lines()
        .filter(|line| line.starts_with("limit-fixture "))
        .collect();
    assert_eq!(
        (lines, survivors.iter().all(|alive| *alive)),
        (
            vec![
                format!("limit-fixture claimed={marked_pid}").as_str(),
                "limit-fixture killed=ok",
                "limit-fixture exhausted=error",
            ],
            true
        )
    );
    Ok(())
}

/// Scans under a lowered descriptor limit; reached only through the test above.
#[test]
fn descriptor_limited_fixture() {
    let Ok(input) = std::env::var("ORA_UTILS_LIMIT_FIXTURE") else {
        return;
    };
    let fields: Vec<_> = input.split(' ').collect();
    let scope = MarkScope::new(
        EnvironmentMark::new(NAME, fields[0]).unwrap_or_else(|error| panic!("{error}")),
        fields[1].parse().unwrap_or_else(|error| panic!("{error}")),
        fields[2].parse().unwrap_or_else(|error| panic!("{error}")),
    );
    let lower = |soft: u64| {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit and setrlimit read and write one valid record for this process only.
        unsafe {
            assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit), 0);
            limit.rlim_cur = soft;
            assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &limit), 0);
        }
    };
    lower(LOWERED_LIMIT);
    let claimed = scope
        .scan()
        .map(|scan| {
            scan.claimed
                .iter()
                .map(|process| process.pid.to_string())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|error| format!("error {error}"));
    println!("limit-fixture claimed={claimed}");
    let killed = scope.kill().map_or("error", |()| "ok");
    println!("limit-fixture killed={killed}");
    // Below the descriptors already open, procfs cannot even be listed.
    lower(/*soft*/ 3);
    let exhausted = scope.scan().map_or("error", |_| "clean");
    println!("limit-fixture exhausted={exhausted}");
}

/// Exits only the main thread, through the raw syscall, while a second thread writes a file
/// after a delay. Arguments: the `SYS_exit` number of this architecture, then the file.
const LEADERLESS: &str = "import ctypes,sys,threading,time
number,path=int(sys.argv[1]),sys.argv[2]
def work():
    time.sleep(3)
    open(path,'w').write('written by a surviving thread')
threading.Thread(target=work).start()
print('ready',flush=True)
ctypes.CDLL(None).syscall(number,0)";

/// A marked process whose leader exited shows `Z` in its stat while a thread still runs; it is
/// claimed through that thread and killed as a whole thread group before it can write.
#[test]
fn a_marked_process_whose_leader_exited_is_claimed_and_killed() -> TestResult {
    let token = token("leaderless")?;
    let scope = MarkScope::new(
        EnvironmentMark::new(NAME, &token)?,
        linux_boot_ticks()?,
        uid(),
    );
    let directory = tempfile::tempdir()?;
    let written = directory.path().join("written");
    let mut guard = Reaped(vec![
        Command::new("python3")
            .args(["-c", LEADERLESS, &libc::SYS_exit.to_string()])
            .arg(&written)
            .env(NAME, &token)
            .stdout(Stdio::piped())
            .spawn()?,
    ]);
    let child = &mut guard.0[0];
    let handle = LinuxPidFd::for_child(child)?;
    let mut ready = String::new();
    BufReader::new(child.stdout.take().ok_or("stdout")?).read_line(&mut ready)?;
    let stat = format!("/proc/{}/stat", child.id());
    until(|| {
        std::fs::read_to_string(&stat).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| rest.starts_with(" Z"))
        })
    });
    let claimed: Vec<_> = scope
        .scan()?
        .claimed
        .iter()
        .map(|process| process.pid)
        .collect();
    scope.kill()?;
    until(|| handle.has_exited().unwrap_or(false));
    child.wait()?;
    std::thread::sleep(Duration::from_secs(/*secs*/ 4));
    assert_eq!((claimed, written.exists()), (vec![child.id()], false));
    Ok(())
}

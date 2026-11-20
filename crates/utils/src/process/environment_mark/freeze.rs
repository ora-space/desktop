//! Freezing and killing what a scan claims, with a wall-clock bound that holds even when a scan
//! hangs.
//!
//! A frozen process can neither fork nor exit, so freezing every claimed process before killing
//! any keeps an environment-cleared child attached to its frozen parent for the next pass. The
//! passes themselves can hang, though: reading one `environ` waits for that process's memory
//! lock, which a stuck FUSE or NFS mount can hold indefinitely. A watchdog therefore shares the
//! frozen set and kills it once the bound passes, so no process stays stopped behind a hung read.

use super::super::linux_process::{LinuxPidFd, ProcessSignal, linux_process};
use super::{ClaimedProcess, MarkScan, scan};
use std::collections::BTreeSet;
use std::io;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Freezing passes before the frozen processes are killed. A pass that finds no process it has
/// not frozen yet ends the loop early; the bound only stops a fork storm from holding the caller.
const FREEZE_PASSES: usize = 32;

/// How long processes may stay frozen. Past it the watchdog kills every frozen process, and the
/// passes still running kill what they find instead of freezing it.
pub(super) const FREEZE_BOUND: Duration = Duration::from_secs(2);

/// The frozen set shared by the passes and the watchdog.
#[derive(Default)]
struct Freeze {
    frozen: BTreeSet<ClaimedProcess>,
    /// Set once the frozen set was killed, by the passes finishing or by the watchdog.
    closed: bool,
}

/// Freezes what each `scan` claims until a pass finds nothing new, then kills every frozen
/// process; after `bound`, kills them whether or not a scan returned. Returns the first scan
/// error, never a signal outcome: only a later scan shows what remains.
pub(super) fn kill_within(
    bound: Duration,
    mut scan: impl FnMut() -> io::Result<MarkScan>,
) -> io::Result<()> {
    let shared = (Mutex::new(Freeze::default()), Condvar::new());
    let (state, woken) = &shared;
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let deadline = Instant::now() + bound;
            let mut freeze = state.lock().unwrap_or_else(PoisonError::into_inner);
            while !freeze.closed {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    close(&mut freeze);
                    break;
                }
                freeze = woken
                    .wait_timeout(freeze, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        });
        let result = passes(state, &mut scan);
        close(&mut state.lock().unwrap_or_else(PoisonError::into_inner));
        woken.notify_all();
        result
    })
}

/// Runs the freezing passes against the shared state.
fn passes(
    state: &Mutex<Freeze>,
    scan: &mut impl FnMut() -> io::Result<MarkScan>,
) -> io::Result<()> {
    // Remembered here as well, so a pass after the watchdog fired still knows what it has seen.
    let mut seen = BTreeSet::new();
    for _ in 0..FREEZE_PASSES {
        let claimed = scan()?.claimed;
        let mut quiet = true;
        for process in claimed {
            if seen.contains(&process) {
                continue;
            }
            quiet = false;
            // Pinned and signalled under the lock, so the watchdog never misses a process frozen
            // just as it fires, and nothing is frozen after it has.
            let mut freeze = state.lock().unwrap_or_else(PoisonError::into_inner);
            match pin(process) {
                Ok(handle) if freeze.closed => {
                    let _ = handle.signal(ProcessSignal::Kill);
                    seen.insert(process);
                }
                Ok(handle) => {
                    let _ = handle.signal(ProcessSignal::Stop);
                    freeze.frozen.insert(process);
                    seen.insert(process);
                }
                // It exited; nothing is left to freeze.
                Err(error) if scan::gone(&error) => {
                    seen.insert(process);
                }
                // Not remembered, so the next pass tries this process again.
                Err(_) => {}
            }
        }
        if quiet {
            break;
        }
    }
    Ok(())
}

/// Kills every frozen process once. A stopped process cannot exit on its own, so each is pinned
/// again by PID and start tick: one pidfd is open at a time, and a PID taken over meanwhile
/// fails the start-tick check.
///
/// A process that cannot be pinned again for another reason stays stopped until a later call.
/// Retrying here would not help: what fails is opening a descriptor, which runs out for the whole
/// Node, not for one process, and spinning under the lock would hold back the passes too. A
/// stopped process is still live and still carries its mark, so every scan claims it until a
/// later call kills it, and evidence never reads it as stopped meanwhile.
fn close(freeze: &mut Freeze) {
    if freeze.closed {
        return;
    }
    freeze.closed = true;
    for process in std::mem::take(&mut freeze.frozen) {
        if let Ok(handle) = pin(process) {
            let _ = handle.signal(ProcessSignal::Kill);
        }
    }
}

/// Pins the claimed process again, refusing a PID that a later process has taken over.
fn pin(process: ClaimedProcess) -> io::Result<LinuxPidFd> {
    let current = linux_process(process.pid)?;
    if current.start_ticks != process.start_ticks {
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    LinuxPidFd::from_observation(&current)
}

#[cfg(test)]
mod tests {
    use super::{ClaimedProcess, MarkScan, kill_within, pin};
    use crate::process::linux_process;
    use pretty_assertions::assert_eq;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A child that is killed and reaped however the test ends.
    struct Sleeper(Child);

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Starts a long sleep and names it as a scan would.
    fn sleeper() -> Result<(Sleeper, ClaimedProcess), Box<dyn std::error::Error>> {
        let child = Sleeper(Command::new("/bin/sleep").arg("60").spawn()?);
        let process = ClaimedProcess {
            pid: child.0.id(),
            start_ticks: linux_process(child.0.id())?.start_ticks,
        };
        Ok((child, process))
    }

    /// The one-letter state of a live process.
    fn state(pid: u32) -> Option<char> {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()?
            .rsplit_once(") ")?
            .1
            .chars()
            .next()
    }

    /// Hands out recorded scans in order, then clean ones.
    fn scans(mut recorded: Vec<MarkScan>) -> impl FnMut() -> std::io::Result<MarkScan> {
        recorded.reverse();
        move || Ok(recorded.pop().unwrap_or_default())
    }

    /// A PID now held by a process with another start tick is neither pinned nor stopped: the
    /// start tick is the only thing telling a reused PID from the claimed process.
    #[test]
    fn a_reused_pid_is_neither_pinned_nor_frozen() -> TestResult {
        let (mut child, process) = sleeper()?;
        let reused = ClaimedProcess {
            start_ticks: process.start_ticks + 1,
            ..process
        };
        let pinned = pin(reused).map(drop).map_err(|error| error.raw_os_error());
        kill_within(
            Duration::from_secs(5),
            scans(vec![MarkScan {
                claimed: vec![reused],
                undecided: 0,
            }]),
        )?;
        let stopped = state(process.pid) == Some('T');
        assert_eq!(
            (pinned, stopped, child.0.try_wait()?.is_none()),
            (Err(Some(libc::ESRCH)), false, true)
        );
        Ok(())
    }

    /// A process frozen by the first pass dies at the bound while the next scan still hangs, so
    /// a hung `environ` read never leaves it stopped.
    #[test]
    fn frozen_processes_die_at_the_bound_while_a_scan_hangs() -> TestResult {
        let (_child, process) = sleeper()?;
        let handle = pin(process)?;
        let bound = Duration::from_millis(200);
        let hang = Duration::from_secs(3);
        let began = Instant::now();
        let killer = std::thread::spawn(move || {
            let mut first = true;
            kill_within(bound, move || {
                if std::mem::take(&mut first) {
                    return Ok(MarkScan {
                        claimed: vec![process],
                        undecided: 0,
                    });
                }
                std::thread::sleep(hang);
                Ok(MarkScan::default())
            })
        });
        while !handle.has_exited()? && began.elapsed() < hang {
            std::thread::sleep(Duration::from_millis(5));
        }
        let died_after = began.elapsed();
        let result = killer.join().map_err(|_| "kill thread panicked")?;
        assert!(result.is_ok());
        assert!(
            died_after < bound + Duration::from_millis(500),
            "{died_after:?}"
        );
        Ok(())
    }

    /// Once the watchdog has fired, a pass kills what it finds instead of freezing it: the frozen
    /// set was already killed, so nothing would kill a process frozen afterwards.
    #[test]
    fn a_pass_after_the_bound_kills_without_freezing() -> TestResult {
        let (_early_child, early) = sleeper()?;
        let (_late_child, late) = sleeper()?;
        let (early_handle, late_handle) = (pin(early)?, pin(late)?);
        let mut scanned = 0;
        kill_within(Duration::from_millis(100), || {
            scanned += 1;
            match scanned {
                1 => Ok(MarkScan {
                    claimed: vec![early],
                    undecided: 0,
                }),
                // Only the watchdog kills `early` while this scan runs, so returning once it died
                // hands `late` to a pass that certainly runs after the bound.
                2 => {
                    let began = Instant::now();
                    while !early_handle.has_exited()? && began.elapsed() < Duration::from_secs(5) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok(MarkScan {
                        claimed: vec![late],
                        undecided: 0,
                    })
                }
                _ => Ok(MarkScan::default()),
            }
        })?;
        let began = Instant::now();
        while !late_handle.has_exited()? && began.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Neither child is reaped yet, so a killed one is a zombie and a frozen one still `T`.
        assert_eq!(
            (
                early_handle.has_exited()?,
                late_handle.has_exited()?,
                state(late.pid)
            ),
            (true, true, Some('Z'))
        );
        Ok(())
    }

    /// Without a hang the passes finish first and kill every frozen process themselves.
    #[test]
    fn frozen_processes_die_when_the_passes_finish() -> TestResult {
        let (_child, process) = sleeper()?;
        let handle = pin(process)?;
        let began = Instant::now();
        kill_within(
            Duration::from_secs(30),
            scans(vec![MarkScan {
                claimed: vec![process],
                undecided: 0,
            }]),
        )?;
        while !handle.has_exited()? {
            assert!(began.elapsed() < Duration::from_secs(5), "still alive");
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

//! Finds every live process one launch started, including those that left its process group or
//! session, through an environment entry the launcher gave each of them.
//!
//! Process groups and sessions are chosen by the processes themselves: a tool shell that calls
//! `setsid` and backgrounds a job hands that job to init, where no group signal reaches it. The
//! environment, by contrast, is inherited across `fork` and `execve` unless a process clears it,
//! so a launcher that puts one entry on every child can find their descendants wherever they
//! moved. A scan reads `/proc/<pid>/environ` of the processes that started no earlier than the
//! launch, through pinned proc directories, so a reused PID is never read or signalled in place
//! of the original.
//!
//! Limits a caller must not present as stronger containment:
//! - a process that clears or overwrites its environment (`env -i`) carries no entry. It is still
//!   claimed while its parent is claimed, through the parent link, but not once it is orphaned;
//! - the entry is not a secret. A process that can read a claimed process's environment can start
//!   processes carrying the entry, and those are claimed too. Only processes that started at or
//!   after the scope's start can be claimed, and writing the entry into an existing process needs
//!   ptrace rights over it, which already include the right to signal it;
//! - a process whose environment the scanner may not read is claimed only through the parent
//!   link. It leaves the scan undecided when it runs as the scope's user while the scanner does
//!   not, because then no environment of the scope is readable (for example a root scanner
//!   without `CAP_SYS_PTRACE`). An unreadable process of the scanner's own user is one that made
//!   itself non-dumpable; counting all of those would let unrelated programs block every stop.

use super::linux_process::{
    LinuxPidFd, LinuxProcessStat, ProcessSignal, linux_process_snapshot, read_pinned,
};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::fs::MetadataExt;

/// Separates the tokens of nested launches inside one value.
const SEPARATOR: u8 = b':';

/// Linux bounds a new program's arguments and environment together to 6 MiB, so a longer block
/// that does not show the entry is not trusted to lack it.
const ENVIRON_LIMIT: usize = 8 << 20;

/// Freezing passes before the frozen processes are killed. A pass that finds no process it has
/// not frozen yet ends the loop early; the bound only stops a fork storm from holding the caller.
const FREEZE_PASSES: usize = 32;

/// One `NAME=token` environment entry a launcher gives every process it starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentMark {
    name: String,
    token: String,
}

impl EnvironmentMark {
    /// Accepts only a name and token that survive one environment entry intact.
    pub fn new(name: impl Into<String>, token: impl Into<String>) -> io::Result<Self> {
        let (name, token) = (name.into(), token.into());
        if name.is_empty()
            || name.contains(['=', '\0'])
            || token.is_empty()
            || token.contains([char::from(SEPARATOR), '\0'])
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid environment mark",
            ));
        }
        Ok(Self { name, token })
    }

    /// The variable a child's environment carries the mark under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value a child receives. A value inherited from an enclosing launch stays in front, so
    /// that launch still finds the children of this one.
    pub fn child_value(&self, inherited: Option<&OsStr>) -> OsString {
        match inherited {
            Some(inherited) if !inherited.is_empty() => {
                let mut value = inherited.to_owned();
                value.push(":");
                value.push(&self.token);
                value
            }
            Some(_) | None => OsString::from(&self.token),
        }
    }

    /// Whether a NUL-separated environment block has an entry whose value lists this token.
    fn is_in(&self, environ: &[u8]) -> bool {
        environ.split(|byte| *byte == 0).any(|entry| {
            entry
                .strip_prefix(self.name.as_bytes())
                .and_then(|rest| rest.strip_prefix(b"="))
                .is_some_and(|value| {
                    value
                        .split(|byte| *byte == SEPARATOR)
                        .any(|token| token == self.token.as_bytes())
                })
        })
    }
}

/// The processes one mark may claim: live ones that started at or after `since_ticks`, in the
/// boot-relative clock ticks of [`linux_boot_ticks`], for a launch whose processes run as `uid`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkScope {
    mark: EnvironmentMark,
    since_ticks: u64,
    uid: u32,
}

/// What one pass over procfs found.
#[derive(Debug)]
pub struct MarkScan {
    /// Live processes that carry the mark or descend from a claimed process.
    pub claimed: Vec<LinuxProcessStat>,
    /// Processes that may belong to the scope but whose environment could not be checked.
    pub undecided: usize,
}

impl MarkScope {
    pub fn new(mark: EnvironmentMark, since_ticks: u64, uid: u32) -> Self {
        Self {
            mark,
            since_ticks,
            uid,
        }
    }

    /// Observes every visible process once. Failing to enumerate procfs, or an unexpected read
    /// failure, is an error rather than an empty scan.
    pub fn scan(&self) -> io::Result<MarkScan> {
        // SAFETY: geteuid only reads this process's credentials.
        let scanner = unsafe { libc::geteuid() };
        let own = std::process::id();
        let mut candidates = Vec::new();
        for observation in linux_process_snapshot()? {
            let process = observation?;
            // Exited processes cannot act any more; their zombies stay until a parent reaps them.
            if process.pid == own
                || process.start_ticks < self.since_ticks
                || matches!(process.state, b'Z' | b'X')
            {
                continue;
            }
            let environ = match read_pinned(&process.directory, c"environ", ENVIRON_LIMIT) {
                Ok((bytes, _)) if self.mark.is_in(&bytes) => Environ::Marked,
                Ok((_, truncated)) if truncated => Environ::Unreadable,
                Ok(_) => Environ::Unmarked,
                Err(error) if gone(&error) => continue,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    Environ::Unreadable
                }
                Err(error) => return Err(error),
            };
            let owner = match process.directory.metadata() {
                Ok(metadata) => metadata.uid(),
                Err(error) if gone(&error) => continue,
                Err(error) => return Err(error),
            };
            candidates.push(Candidate {
                pid: process.pid,
                parent: process.parent,
                owner,
                environ,
                process,
            });
        }
        let (claimed, undecided) = classify(candidates, self.uid, scanner);
        Ok(MarkScan { claimed, undecided })
    }

    /// Freezes every claimed process, rescanning until a pass finds none it has not frozen, and
    /// then kills them all.
    ///
    /// A frozen process can neither fork nor exit, so a child forked just before its parent froze
    /// still names that parent in the next pass even if it cleared its environment; killing
    /// straight away would orphan it first. Signal failures are not reported: only a later scan
    /// shows what remains, so callers decide on evidence, never on this call returning.
    pub fn kill(&self) -> io::Result<()> {
        let mut seen = BTreeSet::new();
        let mut frozen = Vec::new();
        let mut scanned = Ok(());
        for _ in 0..FREEZE_PASSES {
            let scan = match self.scan() {
                Ok(scan) => scan,
                Err(error) => {
                    scanned = Err(error);
                    break;
                }
            };
            let before = seen.len();
            for process in scan.claimed {
                if seen.insert((process.pid, process.start_ticks))
                    && let Ok(handle) = LinuxPidFd::from_observation(&process)
                {
                    let _ = handle.signal(ProcessSignal::Stop);
                    frozen.push(handle);
                }
            }
            if seen.len() == before {
                break;
            }
        }
        // Frozen processes are always killed, even after a failed rescan, so none stays stopped.
        for handle in &frozen {
            let _ = handle.signal(ProcessSignal::Kill);
        }
        scanned
    }
}

/// The current boot-relative time in the clock ticks `/proc/<pid>/stat` reports start times in.
///
/// The kernel derives a start time from the same boot clock and truncates it to ticks, so a
/// process forked after this call never reports an earlier tick.
pub fn linux_boot_ticks() -> io::Result<u64> {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one timespec into the valid record above.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: sysconf takes a scalar name and only reads system configuration.
    let hertz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let (Ok(seconds), Ok(nanoseconds), Ok(hertz)) = (
        u64::try_from(now.tv_sec),
        u64::try_from(now.tv_nsec),
        u64::try_from(hertz),
    ) else {
        return Err(io::Error::other("invalid boot clock"));
    };
    if hertz == 0 || 1_000_000_000 % hertz != 0 {
        return Err(io::Error::other("unsupported clock tick rate"));
    }
    Ok(seconds * hertz + nanoseconds / (1_000_000_000 / hertz))
}

/// What a pinned read of one process's environment showed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Environ {
    Marked,
    Unmarked,
    Unreadable,
}

/// One live process that started inside the scope's time window.
struct Candidate<T> {
    pid: u32,
    parent: u32,
    /// The owner of its proc directory: its user, or root once it made itself non-dumpable.
    owner: u32,
    environ: Environ,
    process: T,
}

/// Claims the marked candidates and, transitively, the children of claimed ones, and counts the
/// unreadable candidates of the scope's user when the scanner runs as another user.
fn classify<T>(candidates: Vec<Candidate<T>>, uid: u32, scanner: u32) -> (Vec<T>, usize) {
    let mut claimed: BTreeSet<u32> = candidates
        .iter()
        .filter(|candidate| candidate.environ == Environ::Marked)
        .map(|candidate| candidate.pid)
        .collect();
    // Parents usually precede their children in PID order, but PIDs wrap; iterate to a fixed
    // point. A parent outside the window is never a candidate, so it never lends its children.
    loop {
        let before = claimed.len();
        for candidate in &candidates {
            if claimed.contains(&candidate.parent) {
                claimed.insert(candidate.pid);
            }
        }
        if claimed.len() == before {
            break;
        }
    }
    let mut undecided = 0;
    let mut found = Vec::new();
    for candidate in candidates {
        if claimed.contains(&candidate.pid) {
            found.push(candidate.process);
        } else if candidate.environ == Environ::Unreadable
            && candidate.owner == uid
            && scanner != uid
        {
            undecided += 1;
        }
    }
    (found, undecided)
}

/// Whether a procfs failure only means the process exited during the scan.
fn gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, Environ, EnvironmentMark, classify};
    use pretty_assertions::assert_eq;
    use std::ffi::OsStr;

    /// Builds one candidate whose payload is its own PID.
    fn candidate(pid: u32, parent: u32, owner: u32, environ: Environ) -> Candidate<u32> {
        Candidate {
            pid,
            parent,
            owner,
            environ,
            process: pid,
        }
    }

    /// An entry matches only under the exact name and as a whole token of a nested list.
    #[test]
    fn marks_match_whole_tokens_under_the_exact_name() -> std::io::Result<()> {
        let mark = EnvironmentMark::new("ORA_MARK", "attempt")?;
        let block = |entries: &[&str]| entries.join("\0").into_bytes();
        assert_eq!(
            [
                block(&["PATH=/bin", "ORA_MARK=attempt"]),
                block(&["ORA_MARK=outer:attempt"]),
                block(&["ORA_MARK=attempt-2"]),
                block(&["ORA_MARK_X=attempt"]),
                block(&["XORA_MARK=attempt"]),
                block(&["ORA_MARK="]),
            ]
            .map(|environ| mark.is_in(&environ)),
            [true, true, false, false, false, false]
        );
        assert_eq!(
            (
                mark.child_value(None),
                mark.child_value(Some(OsStr::new(""))),
                mark.child_value(Some(OsStr::new("outer"))),
            ),
            ("attempt".into(), "attempt".into(), "outer:attempt".into())
        );
        assert_eq!(
            [
                ("", "a"),
                ("A=B", "a"),
                ("A", ""),
                ("A", "a:b"),
                ("A", "a\0")
            ]
            .map(|(name, token)| EnvironmentMark::new(name, token).is_ok()),
            [false; 5]
        );
        Ok(())
    }

    /// Descendants are claimed through their parents whatever their environment shows, while
    /// unrelated processes stay unclaimed and unreadable ones are undecided only for a foreign
    /// scanner.
    #[test]
    fn classification_follows_parent_links_and_counts_foreign_unreadable_processes() {
        let candidates = || {
            vec![
                // A grandchild listed before its parent, as after PID wrap-around.
                candidate(5, 4, 1000, Environ::Unmarked),
                candidate(3, 1, 1000, Environ::Marked),
                candidate(4, 3, 0, Environ::Unreadable),
                candidate(6, 1, 1000, Environ::Unmarked),
                candidate(7, 1, 1000, Environ::Unreadable),
                candidate(8, 2, 0, Environ::Unreadable),
            ]
        };
        assert_eq!(
            (
                classify(candidates(), /*uid*/ 1000, /*scanner*/ 1000),
                classify(candidates(), /*uid*/ 1000, /*scanner*/ 0),
            ),
            ((vec![5, 3, 4], 0), (vec![5, 3, 4], 1))
        );
    }
}

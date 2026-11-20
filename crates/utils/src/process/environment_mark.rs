//! Finds every live process one launch started, including those that left its process group or
//! session, through an environment entry the launcher gave each of them.
//!
//! Process groups and sessions are chosen by the processes themselves: a tool shell that calls
//! `setsid` and backgrounds a job hands that job to init, where no group signal reaches it. The
//! environment, by contrast, is inherited across `fork` and `execve` unless a process clears it,
//! so a launcher that puts one entry on every child can find their descendants wherever they
//! moved. A scan reads `/proc/<pid>/environ` of the processes that started no earlier than the
//! launch, through pinned proc directories, so a reused PID is never read or signalled in place
//! of the original. A pass holds a bounded handful of descriptors whatever the number of
//! processes, and a leader whose own thread exited is still read and claimed through one of its
//! live threads.
//!
//! Limits a caller must not present as stronger containment:
//! - a process that clears or overwrites its environment (`env -i`, or a daemon rewriting its
//!   process title over the environment block) carries no entry. It is still claimed while its
//!   parent is claimed, through the parent link, but not once it is orphaned;
//! - a parent lends its children only while its PID still names the observed process; a PID
//!   reused by an unrelated process during the same pass lends nothing;
//! - the entry is not a secret. A process that can read a claimed process's environment can start
//!   processes carrying the entry, and those are claimed too. Only processes that started at or
//!   after the scope's start can be claimed, and writing the entry into an existing process needs
//!   ptrace rights over it, which already include the right to signal it;
//! - a process whose environment the scanner may not read is claimed only through the parent
//!   link. It leaves the scan undecided when it runs as the scope's user while the scanner does
//!   not, because then no environment of the scope is readable (for example a root scanner
//!   without `CAP_SYS_PTRACE`). An unreadable process of the scanner's own user is one that made
//!   itself non-dumpable; counting all of those would let unrelated programs block every stop.

mod freeze;
mod scan;

use std::ffi::{OsStr, OsString};
use std::io;

/// Separates the tokens of nested launches inside one value.
const SEPARATOR: u8 = b':';

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

/// One claimed process, named by PID and start tick so it can be pinned again without
/// confusing it with a later process that reused the PID.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ClaimedProcess {
    pub pid: u32,
    pub start_ticks: u64,
}

/// What one pass over procfs found.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct MarkScan {
    /// Live processes that carry the mark or descend from a claimed process.
    pub claimed: Vec<ClaimedProcess>,
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

    /// Observes every visible process once, holding a bounded handful of descriptors. Failing
    /// to enumerate procfs, running out of descriptors, or an unexpected read failure is an
    /// error rather than an empty scan.
    pub fn scan(&self) -> io::Result<MarkScan> {
        scan::scan(&self.mark, self.since_ticks, self.uid)
    }

    /// Freezes every claimed process, rescanning until a pass finds none it has not frozen, and
    /// then kills them all.
    ///
    /// A frozen process can neither fork nor exit, so a child forked just before its parent froze
    /// still names that parent in the next pass even if it cleared its environment; killing
    /// straight away would orphan it first. A process whose pidfd could not be opened is tried
    /// again in the next pass of the same call. No process stays frozen longer than two
    /// seconds: past that a watchdog kills every frozen process even while a scan hangs, and
    /// later passes kill instead of freezing. Signal failures are not reported: only a later
    /// scan shows what remains, so callers decide on evidence, never on this call returning.
    pub fn kill(&self) -> io::Result<()> {
        freeze::kill_within(freeze::FREEZE_BOUND, || self.scan())
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

#[cfg(test)]
mod tests {
    use super::EnvironmentMark;
    use pretty_assertions::assert_eq;
    use std::ffi::OsStr;

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
}

//! One pass over procfs: observe each process in the scope's window, then decide which ones the
//! mark claims.
//!
//! A process is observed through a freshly pinned `/proc/<pid>` directory that is closed before
//! the next process is opened, so the pass holds a bounded handful of descriptors whatever the
//! number of processes. Only plain values survive the observation; a claimed process is pinned
//! again by PID and start tick before it is signalled.

use super::super::linux_process::{
    LinuxProcessStat, linux_process, linux_process_snapshot, read_pinned,
};
use super::{ClaimedProcess, EnvironmentMark, MarkScan};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::fs::MetadataExt;

/// Linux bounds a new program's arguments and environment together to 6 MiB, so a longer block
/// that does not show the entry is not trusted to lack it.
const ENVIRON_LIMIT: usize = 8 << 20;

/// What a pinned read of one process's environment showed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Environ {
    Marked,
    Unmarked,
    Unreadable,
}

/// One live process that started inside the scope's time window.
pub(super) struct Candidate {
    pub(super) pid: u32,
    pub(super) parent: u32,
    pub(super) start_ticks: u64,
    /// The owner of its proc directory: its user, or root once it made itself non-dumpable.
    pub(super) owner: u32,
    pub(super) environ: Environ,
}

/// Observes every visible process that started at or after `since_ticks` once.
///
/// Failing to enumerate procfs, running out of descriptors, or any read failure other than the
/// process having exited is an error, never an empty scan.
pub(super) fn scan(mark: &EnvironmentMark, since_ticks: u64, uid: u32) -> io::Result<MarkScan> {
    // SAFETY: geteuid only reads this process's credentials.
    let scanner = unsafe { libc::geteuid() };
    let own = std::process::id();
    let mut candidates = Vec::new();
    for observation in linux_process_snapshot()? {
        // Each observation owns its directory descriptor, released at the end of this iteration.
        let process = observation?;
        if process.pid == own || process.start_ticks < since_ticks {
            continue;
        }
        if let Some(candidate) = observe(mark, &process, scanner)? {
            candidates.push(candidate);
        }
    }
    let (claimed, undecided) = classify(candidates, uid, scanner, |pid, start_ticks| {
        linux_process(pid).is_ok_and(|current| current.start_ticks == start_ticks)
    });
    Ok(MarkScan { claimed, undecided })
}

/// Reads what one process's environment shows; `None` when it turned out to have exited.
fn observe(
    mark: &EnvironmentMark,
    process: &LinuxProcessStat,
    scanner: u32,
) -> io::Result<Option<Candidate>> {
    let owner = match process.directory.metadata() {
        Ok(metadata) => metadata.uid(),
        Err(error) if gone(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    // A leader whose own thread exited reads `Z` while its other threads keep running. Their
    // shared memory still holds the environment, which the leader's own file no longer shows.
    let path = if matches!(process.state, b'Z' | b'X') {
        match live_thread(process.pid)? {
            Some(thread) => CString::new(format!("task/{thread}/environ"))
                .map_err(|_| io::Error::other("invalid thread path"))?,
            None => return Ok(None),
        }
    } else {
        c"environ".to_owned()
    };
    // An unprivileged scanner is always refused another user's environment, so the read is
    // skipped; such a process is still claimed through a claimed parent.
    let environ = if scanner != 0 && owner != scanner {
        Environ::Unreadable
    } else {
        match read_environ(mark, process, &path)? {
            Some(environ) => environ,
            None => return Ok(None),
        }
    };
    Ok(Some(Candidate {
        pid: process.pid,
        parent: process.parent,
        start_ticks: process.start_ticks,
        owner,
        environ,
    }))
}

/// Reads one environment file through the pinned directory; `None` when the process exited.
fn read_environ(
    mark: &EnvironmentMark,
    process: &LinuxProcessStat,
    path: &CStr,
) -> io::Result<Option<Environ>> {
    Ok(Some(
        match read_pinned(&process.directory, path, ENVIRON_LIMIT) {
            Ok((bytes, _)) if mark.is_in(&bytes) => Environ::Marked,
            Ok((_, truncated)) if truncated => Environ::Unreadable,
            Ok(_) => Environ::Unmarked,
            Err(error) if gone(&error) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Environ::Unreadable,
            Err(error) => return Err(error),
        },
    ))
}

/// A thread other than the exited leader, if the thread group still has one.
fn live_thread(pid: u32) -> io::Result<Option<u32>> {
    let threads = match std::fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(threads) => threads,
        Err(error) if gone(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    for thread in threads {
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) if gone(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Some(tid) = thread
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .filter(|tid| *tid != pid)
        {
            return Ok(Some(tid));
        }
    }
    Ok(None)
}

/// Claims the marked candidates and, transitively, the children of claimed ones, and counts the
/// unreadable candidates of the scope's user when the scanner runs as another user.
///
/// A parent lends its children only while `same(pid, start_ticks)` confirms it is still the
/// process the pass observed: if it exited and its PID went to an unrelated process during the
/// pass, a child of that newcomer must not be claimed.
pub(super) fn classify(
    candidates: Vec<Candidate>,
    uid: u32,
    scanner: u32,
    mut same: impl FnMut(u32, u64) -> bool,
) -> (Vec<ClaimedProcess>, usize) {
    let starts: BTreeMap<u32, u64> = candidates
        .iter()
        .map(|candidate| (candidate.pid, candidate.start_ticks))
        .collect();
    let mut claimed: BTreeSet<u32> = candidates
        .iter()
        .filter(|candidate| candidate.environ == Environ::Marked)
        .map(|candidate| candidate.pid)
        .collect();
    let mut verified = BTreeMap::new();
    // Parents usually precede their children in PID order, but PIDs wrap; iterate to a fixed
    // point. A parent outside the window is never a candidate, so it never lends its children.
    loop {
        let before = claimed.len();
        for candidate in &candidates {
            if claimed.contains(&candidate.pid) || !claimed.contains(&candidate.parent) {
                continue;
            }
            let parent = candidate.parent;
            let still = *verified.entry(parent).or_insert_with(|| {
                starts
                    .get(&parent)
                    .is_some_and(|start| same(parent, *start))
            });
            if still {
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
            found.push(ClaimedProcess {
                pid: candidate.pid,
                start_ticks: candidate.start_ticks,
            });
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
pub(super) fn gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, ClaimedProcess, Environ, classify};
    use pretty_assertions::assert_eq;

    /// Builds one candidate whose start tick is its PID.
    fn candidate(pid: u32, parent: u32, owner: u32, environ: Environ) -> Candidate {
        Candidate {
            pid,
            parent,
            start_ticks: u64::from(pid),
            owner,
            environ,
        }
    }

    /// The PIDs of claimed processes, in candidate order.
    fn pids(claimed: Vec<ClaimedProcess>) -> Vec<u32> {
        claimed.into_iter().map(|process| process.pid).collect()
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
        let (same_user, same_user_undecided) = classify(
            candidates(),
            /*uid*/ 1000,
            /*scanner*/ 1000,
            |_, _| true,
        );
        let (foreign, foreign_undecided) = classify(
            candidates(),
            /*uid*/ 1000,
            /*scanner*/ 0,
            |_, _| true,
        );
        assert_eq!(
            (
                pids(same_user),
                same_user_undecided,
                pids(foreign),
                foreign_undecided
            ),
            (vec![5, 3, 4], 0, vec![5, 3, 4], 1)
        );
    }

    /// A parent whose PID now belongs to another process lends nothing: its "children" in the
    /// pass are that newcomer's, and only the marked parent itself stays claimed.
    #[test]
    fn a_parent_replaced_during_the_pass_lends_no_children() {
        let (claimed, undecided) = classify(
            vec![
                candidate(3, 1, 1000, Environ::Marked),
                candidate(9, 3, 1000, Environ::Unmarked),
            ],
            /*uid*/ 1000,
            /*scanner*/ 1000,
            |pid, _| pid != 3,
        );
        assert_eq!((pids(claimed), undecided), (vec![3], 0));
    }
}
